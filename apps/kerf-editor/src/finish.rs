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

use crate::commands::RunCommand;
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::model::{self, RenderJob};
use crate::motion;
use crate::session::{BridgeEvent, Session};
use crate::store::Change;
use crate::theme::{self, height, motion as scene, radius, text};
use crate::ui::{self, icon_button};
use gpui::{
    actions, anchored, canvas, deferred, div, point, prelude::*, px, AnimationExt, AnyElement, App,
    Bounds, Context, Corner, DismissEvent, Edges, ElementId, Entity, EntityId, EventEmitter,
    FocusHandle, Focusable, FontWeight, Global, KeyBinding, MouseMoveEvent, PathBuilder, Pixels,
    Rgba, SharedString, Subscription, Window,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

actions!(finish, [Close]);

/// The key contexts Finish and the chip's menu answer Esc in.
const CONTEXT: &str = "Finish";
const MENU_CONTEXT: &str = "CollabMenu";

/// The panel's size; the workspace rounds its corners.
const WIDTH: f32 = 860.0;
const HEIGHT: f32 = 600.0;
/// Its title row, and the column of sections down its side.
const HEAD: f32 = 48.0;
const NAV: f32 = 196.0;
/// A section's edges.
const PAGE_SIDE: f32 = 24.0;
const PAGE_TOP: f32 = 20.0;
/// The loudness chart's height, and the loudness it spans (LUFS).
const CHART_HEIGHT: f32 = 168.0;
const CHART_TOP: f32 = -6.0;
const CHART_BOTTOM: f32 = -54.0;
/// The levels the chart draws a hairline at, beside the target's.
const CHART_LEVELS: [f32; 3] = [-12.0, -36.0, -48.0];
/// The most points the chart draws, whatever the programme's length.
const CHART_POINTS: usize = 480;
/// The most findings listed; the rest are counted.
const FINDINGS_MOST: usize = 300;
/// How far integrated loudness may be from its target, in LU.
const TOLERANCE: f64 = 1.0;
/// What R128 asks of a programme, until a measurement says otherwise.
const TARGET: f64 = -23.0;
const PEAK_LIMIT: f64 = -1.0;
/// Where the master and the review copy go (master_output's rule).
use crate::session::DELIVERIES;
/// The chip's menu.
const MENU_WIDTH: f32 = 284.0;

// ------------------------------------------------------------ what is kept

/// The last check's report, as `check {status: done}` sent it.
struct Report {
    /// The document revision checked.
    revision: u64,
    valid: bool,
    summary: Summary,
    findings: Vec<Finding>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Summary {
    passed: usize,
    total: usize,
    blocking: usize,
    rows: Vec<Row>,
}

/// One check of the summary: what it found against what it allows.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Row {
    code: String,
    name: String,
    passed: bool,
    value: String,
    limit: Option<String>,
    /// Where its first failing finding starts, in seconds.
    at: Option<f64>,
    blocking: usize,
}

impl Row {
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

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Finding {
    severity: String,
    message: String,
    at: Option<f64>,
    timecode: Option<String>,
}

impl Finding {
    /// Errors block delivery; warnings want a look; information is only said.
    fn rank(&self) -> u8 {
        match self.severity.as_str() {
            "error" => 0,
            "warning" => 1,
            _ => 2,
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
    fn from_value(event: &Value, revision: u64) -> Self {
        let summary: Summary = serde_json::from_value(event["summary"].clone()).unwrap_or_default();
        let mut findings: Vec<Finding> = event["findings"]
            .as_array()
            .map(|findings| {
                findings
                    .iter()
                    .filter_map(|finding| serde_json::from_value(finding.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        // What blocks first, then what wants a look; each in the order found.
        findings.sort_by_key(Finding::rank);
        Self {
            revision,
            valid: event["valid"].as_bool().unwrap_or(summary.blocking == 0),
            summary,
            findings,
        }
    }
}

/// The last measurement of the programme's loudness (`loudness {status: ready}`).
struct Loudness {
    integrated: Option<f64>,
    lra: Option<f64>,
    true_peak: Option<f64>,
    short_term_max: Option<f64>,
    duration: f64,
    target: f64,
    limit: f64,
    revision: u64,
    points: Rc<Vec<ChartPoint>>,
}

/// One point of the chart: its time, its place across (0..1), and the
/// momentary and short-term loudness there (None for silence).
#[derive(Clone, Copy, Debug)]
struct ChartPoint {
    at: f32,
    x: f32,
    momentary: Option<f32>,
    short: Option<f32>,
}

impl Loudness {
    fn from_value(event: &Value) -> Self {
        let series: Vec<(f32, Option<f32>, Option<f32>)> = event["series"]
            .as_array()
            .map(|series| {
                series
                    .iter()
                    .filter_map(|sample| {
                        let sample = sample.as_array()?;
                        let at = sample.first()?.as_f64()? as f32;
                        let level = |index: usize| {
                            sample.get(index).and_then(Value::as_f64).map(|v| v as f32)
                        };
                        Some((at, level(1), level(2)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let last = series
            .last()
            .map(|sample| f64::from(sample.0))
            .unwrap_or(0.0);
        let duration = event["duration"]
            .as_f64()
            .filter(|d| *d > 0.0)
            .unwrap_or(last);
        Self {
            integrated: event["integrated"].as_f64(),
            lra: event["lra"].as_f64(),
            true_peak: event["truePeakMax"].as_f64(),
            short_term_max: event["shortTermMax"].as_f64(),
            duration,
            target: event["target"].as_f64().unwrap_or(TARGET),
            limit: event["limit"].as_f64().unwrap_or(PEAK_LIMIT),
            revision: event["revision"].as_u64().unwrap_or(0),
            points: Rc::new(chart_points(&series, duration as f32)),
        }
    }
}

/// `series` (a sample each 0.1 s) in at most [`CHART_POINTS`] points, each
/// the mean of the loudness its samples heard.
fn chart_points(series: &[(f32, Option<f32>, Option<f32>)], duration: f32) -> Vec<ChartPoint> {
    if series.is_empty() {
        return Vec::new();
    }
    let size = series.len().div_ceil(CHART_POINTS).max(1);
    let mean = |values: &mut dyn Iterator<Item = f32>| {
        let (sum, count) = values.fold((0.0, 0), |(sum, count), value| (sum + value, count + 1));
        (count > 0).then(|| sum / count as f32)
    };
    series
        .chunks(size)
        .map(|chunk| {
            let at = chunk[0].0;
            ChartPoint {
                at,
                x: if duration > 0.0 {
                    (at / duration).clamp(0.0, 1.0)
                } else {
                    0.0
                },
                momentary: mean(&mut chunk.iter().filter_map(|sample| sample.1)),
                short: mean(&mut chunk.iter().filter_map(|sample| sample.2)),
            }
        })
        .collect()
}

/// Where the last captions went, or why they did not.
struct Captions {
    format: String,
    outcome: Result<String, String>,
}

/// What Finish knows of a project: the check running or the last one's
/// report, the loudness measured, the captions written. One for each open
/// project, kept while its session lives, so Finish can close and open
/// again without losing a check.
struct Finishing {
    /// A check's progress while it runs.
    checking: Option<f64>,
    /// The revision the running check reads.
    check_revision: Option<u64>,
    report: Option<Report>,
    check_failed: Option<String>,
    measuring: Option<f64>,
    loudness: Option<Loudness>,
    loudness_failed: Option<String>,
    captions: Option<Captions>,
    /// Why the last render could not be queued, or failed.
    export_failed: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// The [`Finishing`] of every open project, by its session.
#[derive(Default)]
struct Kept(HashMap<EntityId, Entity<Finishing>>);

impl Global for Kept {}

/// The project's [`Finishing`], made the first time it is asked for.
fn finishing(session: &Entity<Session>, cx: &mut App) -> Entity<Finishing> {
    let id = session.entity_id();
    if let Some(kept) = cx.try_global::<Kept>().and_then(|kept| kept.0.get(&id)) {
        return kept.clone();
    }
    let made = cx.new(|cx| Finishing::new(session, cx));
    cx.default_global::<Kept>().0.insert(id, made.clone());
    made
}

impl Finishing {
    fn new(session: &Entity<Session>, cx: &mut Context<Self>) -> Self {
        let id = session.entity_id();
        let subscriptions = vec![
            cx.subscribe(session, |this, session, event: &BridgeEvent, cx| {
                let revision = session.read(cx).revision();
                if this.apply(&event.0, revision) {
                    cx.notify();
                }
            }),
            // Gone with its project.
            cx.observe_release(session, move |_, _, cx| {
                if cx.has_global::<Kept>() {
                    cx.update_global::<Kept, _>(|kept, _| {
                        kept.0.remove(&id);
                    });
                }
            }),
        ];
        Self {
            checking: None,
            check_revision: None,
            report: None,
            check_failed: None,
            measuring: None,
            loudness: None,
            loudness_failed: None,
            captions: None,
            export_failed: None,
            _subscriptions: subscriptions,
        }
    }

    /// Folds one bridge event in; true when it changed anything shown.
    fn apply(&mut self, event: &Value, revision: u64) -> bool {
        let status = event["status"].as_str().unwrap_or_default();
        let message = |fallback: &str| event["message"].as_str().unwrap_or(fallback).to_owned();
        match event["type"].as_str().unwrap_or_default() {
            "check" => match status {
                "running" => {
                    // A check another view started reads the project as it is now.
                    self.check_revision.get_or_insert(revision);
                    self.checking = Some(event["progress"].as_f64().unwrap_or(0.0));
                    self.check_failed = None;
                }
                "done" => {
                    let revision = self.check_revision.take().unwrap_or(revision);
                    self.checking = None;
                    self.check_failed = None;
                    self.report = Some(Report::from_value(event, revision));
                }
                "failed" => {
                    self.checking = None;
                    self.check_revision = None;
                    self.check_failed = Some(message("The check failed"));
                }
                _ => return false,
            },
            "loudness" => match status {
                "running" => self.measuring = Some(event["progress"].as_f64().unwrap_or(0.0)),
                "ready" => {
                    self.measuring = None;
                    self.loudness_failed = None;
                    self.loudness = Some(Loudness::from_value(event));
                }
                "failed" => {
                    self.measuring = None;
                    self.loudness_failed = Some(message("The programme could not be measured"));
                }
                _ => return false,
            },
            "export" if event["kind"] == "captions" => {
                let format = event["format"].as_str().unwrap_or_default().to_owned();
                let outcome = match status {
                    "completed" => Ok(event["path"].as_str().unwrap_or_default().to_owned()),
                    "failed" => Err(message("The captions could not be written")),
                    _ => return false,
                };
                self.captions = Some(Captions { format, outcome });
            }
            "export" => match status {
                "queued" => self.export_failed = None,
                "failed" => self.export_failed = Some(message("The render failed")),
                _ => return false,
            },
            "error" if event["command"] == "export" => {
                self.export_failed = Some(message("The render was not queued"));
            }
            _ => return false,
        }
        true
    }
}

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

    /// Starts Kerf's QC pass, unless one runs.
    fn run_check(&mut self, cx: &mut Context<Self>) {
        if self.finishing.read(cx).checking.is_some() {
            return;
        }
        let revision = self.session.read(cx).revision();
        let sent = self
            .session
            .update(cx, |session, _| session.send(json!({"type": "check"})));
        if sent {
            self.finishing.update(cx, |finishing, cx| {
                finishing.checking = Some(0.0);
                finishing.check_revision = Some(revision);
                finishing.check_failed = None;
                cx.notify();
            });
        }
    }

    /// Measures the programme to R128; unchanged since the last time, the
    /// bridge answers at once.
    fn measure(&mut self, cx: &mut Context<Self>) {
        if self.finishing.read(cx).measuring.is_some() {
            return;
        }
        let sent = self
            .session
            .update(cx, |session, _| session.send(json!({"type": "loudness"})));
        if sent {
            self.finishing.update(cx, |finishing, cx| {
                finishing.measuring = Some(0.0);
                finishing.loudness_failed = None;
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

    /// Renders the master (`master`) or a review copy (`review`) into
    /// deliveries/, named after the project, as master_output's rule has it.
    fn deliver(&mut self, quality: &'static str, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, _| session.deliver(quality));
    }

    fn captions(&mut self, format: &'static str, cx: &mut Context<Self>) {
        self.session.update(cx, |session, _| {
            session.send(json!({"type": "export_captions", "format": format}));
        });
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
                    .tooltip("Close · Esc")
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

    fn page(&self, cx: &Context<Self>) -> impl IntoElement {
        let section = self.section;
        let content = match section {
            Section::Qc => self.qc(cx),
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
        says: &'static str,
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
                    .child(ui::label(says, text::CONTROL, theme::ink3())),
            )
            .children(action)
            .into_any_element()
    }

    // ---------------------------------------------------------- QC

    fn qc(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let finishing = self.finishing.read(cx);
        let running = finishing.checking;
        let revision = self.session.read(cx).revision();
        let run = ui::button(
            "finish-run-qc",
            if finishing.report.is_some() {
                "Run QC Again"
            } else {
                "Run QC"
            },
        )
        .primary()
        .disabled(running.is_some() || !self.connected(cx))
        .on_click(cx.listener(|this, _, _, cx| this.run_check(cx)))
        .into_any_element();
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
        out.push(self.qc_rows(report, cx));
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

    fn qc_rows(&self, report: &Report, cx: &Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(theme::line())
            .overflow_hidden()
            .children(report.summary.rows.iter().enumerate().map(|(index, row)| {
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
                    .when_some(seek, |line, at| {
                        line.cursor_pointer()
                            .hover(|style| style.bg(theme::raised()))
                            .on_click(cx.listener(move |this, _, _, cx| this.seek(at, cx)))
                    })
            }))
            .into_any_element()
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
                                        .pt(px(2.0)),
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

    // ---------------------------------------------------------- loudness

    fn loudness(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let finishing = self.finishing.read(cx);
        let running = finishing.measuring;
        let revision = self.session.read(cx).revision();
        let measure = ui::button(
            "finish-measure",
            if finishing.loudness.is_some() {
                "Measure Again"
            } else {
                "Measure"
            },
        )
        .primary()
        .disabled(running.is_some() || !self.connected(cx))
        .on_click(cx.listener(|this, _, _, cx| this.measure(cx)))
        .into_any_element();
        let mut out = vec![self.title(
            "Loudness",
            "The programme measured to EBU R128: integrated loudness on −23 LUFS, true peak at most −1 dBTP.",
            Some(measure),
        )];
        if let Some(progress) = running {
            out.push(working("Measuring…", progress));
        }
        if let Some(failed) = &finishing.loudness_failed {
            out.push(said(failed.clone(), theme::neg()));
        }
        let Some(loudness) = &finishing.loudness else {
            if running.is_none() && finishing.loudness_failed.is_none() {
                out.push(empty("Not measured yet", "Measure listens to the whole programme; the answer is kept until the next edit."));
            }
            return out;
        };
        out.push(loudness_stats(loudness));
        out.push(self.chart(loudness, cx));
        out.push(
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S4))
                .child(ui::label(
                    format!("Measured at revision {}", loudness.revision),
                    text::META,
                    theme::ink3(),
                ))
                .when(loudness.revision != revision, |row| {
                    row.child(ui::label(
                        "· the project changed since",
                        text::META,
                        theme::warn(),
                    ))
                })
                .into_any_element(),
        );
        out
    }

    fn chart(&self, loudness: &Loudness, cx: &Context<Self>) -> AnyElement {
        let points = loudness.points.clone();
        let target = loudness.target as f32;
        let hover = self.hover.filter(|_| !points.is_empty());
        let hovered = hover.and_then(|x| {
            let index = ((x * (points.len() - 1) as f32).round() as usize).min(points.len() - 1);
            points.get(index).copied()
        });
        let level_y = |level: f32| (CHART_TOP - level) / (CHART_TOP - CHART_BOTTOM) * CHART_HEIGHT;
        let tick = |level: f32| {
            ui::label(number(f64::from(level), 0), text::MONO_SMALL, theme::ink3())
                .absolute()
                .left(px(theme::space::S6))
                .top(px(level_y(level) - 16.0))
        };
        let readout = hovered.map(|point| {
            let level = |value: Option<f32>| {
                value.map_or("—".to_owned(), |v| {
                    format!("{} LUFS", number(f64::from(v), 1))
                })
            };
            format!(
                "{} · Short-term {} · Momentary {}",
                clock(f64::from(point.at)),
                level(point.short),
                level(point.momentary)
            )
        });
        let bounds = self.chart_bounds.clone();
        let painted = self.chart_bounds.clone();
        let hover_x = hovered.map(|point| point.x);
        div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S6))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S12))
                    .child(ui::label(
                        "Over the programme",
                        text::META_STRONG,
                        theme::ink3(),
                    ))
                    .child(div().flex_1())
                    .child(legend(swatch(momentary_ink(), 1.0, false), "Momentary"))
                    .child(legend(
                        swatch(theme::accent_ink(), 2.0, false),
                        "Short-term",
                    ))
                    .child(legend(
                        swatch(target_ink(), 1.0, true),
                        format!("Target {}", number(f64::from(target), 0)),
                    )),
            )
            .child(
                div()
                    .id("finish-chart")
                    .relative()
                    .h(px(CHART_HEIGHT))
                    .w_full()
                    .rounded(px(radius::CARD))
                    .bg(theme::well())
                    .border_1()
                    .border_color(theme::line())
                    .overflow_hidden()
                    .child(
                        canvas(
                            move |area, _, _| painted.set(Some(area)),
                            move |area, _, window, _| {
                                paint_chart(area, &points, target, hover_x, window)
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .children(CHART_LEVELS.iter().map(|level| tick(*level)))
                    .child(tick(target))
                    .when_some(readout, |chart, readout| {
                        chart.child(
                            ui::label(readout, text::MONO_SMALL, theme::ink())
                                .absolute()
                                .top(px(theme::space::S6))
                                .right(px(theme::space::S8))
                                .px(px(theme::space::S6))
                                .rounded(px(radius::SMALL))
                                .bg(theme::with_alpha(theme::card(), 0.9)),
                        )
                    })
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        let Some(area) = bounds.get() else {
                            return;
                        };
                        let across = (event.position.x - area.origin.x) / area.size.width;
                        let inside = area.contains(&event.position);
                        let next = inside.then_some(across.clamp(0.0, 1.0));
                        if next != this.hover {
                            this.hover = next;
                            cx.notify();
                        }
                    }))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !*hovered && this.hover.take().is_some() {
                            cx.notify();
                        }
                    })),
            )
            .child(
                div()
                    .flex()
                    .child(ui::label("0:00", text::MONO_SMALL, theme::ink3()))
                    .child(div().flex_1())
                    .child(ui::label(
                        model::length(loudness.duration),
                        text::MONO_SMALL,
                        theme::ink3(),
                    )),
            )
            .into_any_element()
    }

    // ---------------------------------------------------------- deliver

    fn deliver_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
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
        let latest = |quality: &str| jobs.iter().find(|job| job.quality == quality).cloned();
        let mut out = vec![self.title(
            "Deliver",
            "The master and a review copy go to deliveries/ in the project, numbered when one is there already.",
            None,
        )];
        if let Some(report) = finishing
            .report
            .as_ref()
            .filter(|report| report.summary.blocking > 0)
        {
            out.push(
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
                                report.summary.blocking,
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
                    .into_any_element(),
            );
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
        let captions_said = finishing
            .captions
            .as_ref()
            .map(|captions| match &captions.outcome {
                Ok(path) => {
                    let shown = shown_path(folder.as_deref(), path);
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
        let card = div()
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(theme::line())
            .child(
                Deliverable {
                    name: "Master",
                    makes: "ProRes 422 HQ in QuickTime, with 24-bit sound",
                    goes: named("master", "mov"),
                    newest: latest("master"),
                    problem: finishing.export_failed.clone(),
                }
                .row(master, folder.as_deref(), true),
            )
            .child(
                Deliverable {
                    name: "Review copy",
                    makes: "H.264 at 12 Mb/s, full size, to send round",
                    goes: named("draft", "mp4"),
                    newest: latest("review"),
                    problem: None,
                }
                .row(review, folder.as_deref(), false),
            )
            .child(
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
                    .children(captions_said),
            );
        out.push(card.into_any_element());
        out.push(self.queue(&jobs, folder.as_deref(), cx));
        out
    }

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
            .children(jobs.iter().take(5).enumerate().map(|(index, job)| {
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
                } else if job.status == "completed" && !job.output_path.is_empty() {
                    let path = job.output_path.clone();
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
                                    shown_path(folder, &job.output_path),
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
            }))
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, _, cx| this.dismiss(cx)))
            .on_action(cx.listener(Self::command))
            // A press on the dim around the panel gives the window back.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss(cx)))
            .w(px(WIDTH))
            .h(px(HEIGHT))
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
                    .h(px(HEIGHT - HEAD - 1.0))
                    .child(self.nav(cx))
                    .child(self.page(cx)),
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

/// The three numbers R128 delivers on, each with what it is held to.
fn loudness_stats(loudness: &Loudness) -> AnyElement {
    let integrated = loudness.integrated.map(|value| {
        let off = value - loudness.target;
        if off.abs() <= TOLERANCE {
            (
                theme::pos(),
                format!("On target, {} LUFS", number(loudness.target, 0)),
            )
        } else {
            let way = if off > 0.0 { "over" } else { "under" };
            (
                theme::neg(),
                format!(
                    "{} LU {way} {} LUFS",
                    number(off.abs(), 1),
                    number(loudness.target, 0)
                ),
            )
        }
    });
    let peak = loudness.true_peak.map(|value| {
        if value <= loudness.limit {
            (
                theme::pos(),
                format!("Within {} dBTP", number(loudness.limit, 0)),
            )
        } else {
            (
                theme::neg(),
                format!(
                    "{} dB over {} dBTP",
                    number(value - loudness.limit, 1),
                    number(loudness.limit, 0)
                ),
            )
        }
    });
    let range_note = loudness.short_term_max.map(|max| {
        (
            theme::ink3(),
            format!("Short-term at most {} LUFS", number(max, 1)),
        )
    });
    div()
        .flex()
        .gap(px(theme::space::S12))
        .child(stat("Integrated", loudness.integrated, "LUFS", integrated))
        .child(stat("True peak", loudness.true_peak, "dBTP", peak))
        .child(stat("Loudness range", loudness.lra, "LU", range_note))
        .into_any_element()
}

fn stat(
    name: &'static str,
    value: Option<f64>,
    unit: &'static str,
    note: Option<(Rgba, String)>,
) -> AnyElement {
    let shown = value.map_or("—".to_owned(), |value| number(value, 1));
    div()
        .flex_1()
        .min_w_0()
        .px(px(theme::space::S12))
        .py(px(theme::space::S12))
        .flex()
        .flex_col()
        .gap(px(theme::space::S4))
        .rounded(px(radius::CARD))
        .bg(theme::well())
        .border_1()
        .border_color(theme::line())
        .child(ui::label(name, text::META, theme::ink3()))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(theme::space::S4))
                .child(ui::label(shown, text::MONO_LARGE, theme::ink()))
                .when(value.is_some(), |row| {
                    row.child(ui::label(unit, text::META, theme::ink2()))
                }),
        )
        .child(match (value, note) {
            (None, _) => ui::label("Silent", text::META, theme::ink3()).into_any_element(),
            (Some(_), Some((color, words))) => div()
                .flex()
                .items_center()
                .gap(px(theme::space::S6))
                .child(ui::dot(color, 6.0))
                .child(ui::line(words, text::META, theme::ink2()).min_w_0())
                .into_any_element(),
            (Some(_), None) => div().into_any_element(),
        })
        .into_any_element()
}

/// Momentary loudness: short-term's colour, faint, as the finer grain of
/// the same measure.
fn momentary_ink() -> Rgba {
    theme::with_alpha(theme::accent_ink(), 0.6)
}

/// The target: recessive, a reference and not a series.
fn target_ink() -> Rgba {
    theme::ink3()
}

/// A line of the chart's legend: its mark, then its name in ink.
fn legend(mark: impl IntoElement, words: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(theme::space::S6))
        .child(mark)
        .child(ui::label(words, text::META, theme::ink2()))
}

/// A short line in a series' colour and weight, dashed for the target.
fn swatch(color: Rgba, weight: f32, dashed: bool) -> impl IntoElement {
    let piece = move |width: f32| div().w(px(width)).h(px(weight)).rounded_full().bg(color);
    if dashed {
        div()
            .flex()
            .gap(px(2.0))
            .child(piece(5.0))
            .child(piece(5.0))
            .child(piece(4.0))
    } else {
        div().flex().child(piece(18.0))
    }
}

/// Paints the chart in `area`: hairlines at a few levels, the target
/// dashed, momentary loudness thin and faint, short-term over it, and a
/// line where the pointer is. Silence breaks a line.
fn paint_chart(
    area: Bounds<Pixels>,
    points: &[ChartPoint],
    target: f32,
    hover: Option<f32>,
    window: &mut Window,
) {
    let height = f32::from(area.size.height);
    let width = f32::from(area.size.width);
    let y = |level: f32| {
        let share = ((CHART_TOP - level) / (CHART_TOP - CHART_BOTTOM)).clamp(0.0, 1.0);
        area.origin.y + px(height * share)
    };
    let x = |share: f32| area.origin.x + px(width * share);
    let across = |builder: &mut PathBuilder, level: f32| {
        builder.move_to(point(area.origin.x, y(level)));
        builder.line_to(point(area.origin.x + area.size.width, y(level)));
    };
    let mut grid = PathBuilder::stroke(px(1.0));
    for level in CHART_LEVELS {
        across(&mut grid, level);
    }
    if let Ok(path) = grid.build() {
        window.paint_path(path, theme::line());
    }
    let mut goal = PathBuilder::stroke(px(1.0)).dash_array(&[px(4.0), px(3.0)]);
    across(&mut goal, target);
    if let Ok(path) = goal.build() {
        window.paint_path(path, target_ink());
    }
    let series =
        |pick: fn(&ChartPoint) -> Option<f32>, weight: f32, color: Rgba, window: &mut Window| {
            let mut builder = PathBuilder::stroke(px(weight));
            let mut drawn = false;
            let mut down = false;
            for point_at in points {
                match pick(point_at) {
                    Some(level) => {
                        let at = point(x(point_at.x), y(level));
                        if down {
                            builder.line_to(at);
                            drawn = true;
                        } else {
                            builder.move_to(at);
                        }
                        down = true;
                    }
                    None => down = false,
                }
            }
            if drawn {
                if let Ok(path) = builder.build() {
                    window.paint_path(path, color);
                }
            }
        };
    series(|p| p.momentary, 1.0, momentary_ink(), window);
    series(|p| p.short, 2.0, theme::accent_ink(), window);
    if let Some(share) = hover {
        let mut cursor = PathBuilder::stroke(px(1.0));
        cursor.move_to(point(x(share), area.origin.y));
        cursor.line_to(point(x(share), area.origin.y + area.size.height));
        if let Ok(path) = cursor.build() {
            window.paint_path(path, theme::lineh());
        }
    }
}

/// What Deliver renders: its name, what it makes and where it goes, its
/// newest render, and why the last one did not go.
struct Deliverable {
    name: &'static str,
    makes: &'static str,
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
                shown_path(folder, &job.output_path),
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
    match job.quality.as_str() {
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

/// Three bars of a meter, as an icon is drawn: loudness.
fn meter_glyph(color: Rgba) -> impl IntoElement {
    let bar = move |tall: f32| div().w(px(2.0)).h(px(tall)).rounded_full().bg(color);
    div()
        .size(px(16.0))
        .flex_none()
        .flex()
        .items_end()
        .justify_center()
        .gap(px(2.0))
        .pb(px(2.5))
        .child(bar(6.0))
        .child(bar(11.0))
        .child(bar(8.0))
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

/// `m:ss.s`: a moment of the programme to the tenth.
fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let minutes = (seconds / 60.0).floor();
    format!("{}:{:04.1}", minutes as u64, seconds - minutes * 60.0)
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

// ------------------------------------------------------------------ Collab

/// Where the project stands against its repository (`collab {status}`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct GitStatus {
    repository: bool,
    /// The repository the project is inside of, without being its own.
    enclosing: Option<String>,
    branch: Option<String>,
    remote: Option<String>,
    ahead: u32,
    behind: u32,
    changed: bool,
    conflicted: bool,
    branches: Vec<String>,
    /// The GitHub CLI is installed and signed in.
    gh: bool,
    live: bool,
    busy: bool,
    user: Option<GitUser>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct GitUser {
    name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Closed,
    Open,
    /// Open, with the branches to switch to unfolded.
    Branches,
    Prompt(Prompt),
}

/// What the menu asks words for before it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prompt {
    Publish,
    Commit,
    NewBranch,
    PullRequest,
}

impl Prompt {
    fn title(self) -> &'static str {
        match self {
            Prompt::Publish => "Publish to GitHub",
            Prompt::Commit => "Commit",
            Prompt::NewBranch => "New Branch",
            Prompt::PullRequest => "Open Pull Request",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Prompt::Publish => "Repository name",
            Prompt::Commit => "What changed",
            Prompt::NewBranch => "Branch name",
            Prompt::PullRequest => "Title",
        }
    }

    fn says(self) -> &'static str {
        match self {
            Prompt::Publish => "A private repository on GitHub, made with gh. Left empty, it is named after the project's folder.",
            Prompt::Commit => "Commits your edits without syncing. Left empty, the message describes them.",
            Prompt::NewBranch => "Commits your edits, then makes a branch from here and switches to it. The next sync publishes it.",
            Prompt::PullRequest => "Pushes the branch and opens a pull request on it. Left empty, the title is the branch's name.",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Prompt::Publish => "Publish",
            Prompt::Commit => "Commit",
            Prompt::NewBranch => "Create Branch",
            Prompt::PullRequest => "Open",
        }
    }
}

/// The project's standing against its Git repository, in the toolbar, with
/// Sync and the rest of sharing in its menu.
pub struct Collab {
    session: Entity<Session>,
    status: Option<GitStatus>,
    /// The step a sync is at (committing, fetching…) while it runs.
    stage: Option<String>,
    /// Why the last sync failed, until one succeeds.
    failure: Option<String>,
    /// What the last sync did.
    report: Option<String>,
    /// Media outside the project folder, as the bridge last said.
    external: Option<usize>,
    menu: Menu,
    /// How many times the menu has opened, so each opening fades in.
    menus_opened: u64,
    input: Entity<TextInput>,
    focus_handle: FocusHandle,
    /// What had the keys before the menu took them, to give them back.
    previous_focus: Option<FocusHandle>,
    /// When the work under way began, for the spinner.
    busy_since: Instant,
    /// A person asked for the work under way, so its end is said.
    asked: bool,
    _subscriptions: Vec<Subscription>,
}

impl Collab {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_close(cx);
        let input = cx.new(|cx| TextInput::new(cx, ""));
        let subscriptions =
            vec![
                cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                    this.apply(&event.0, cx)
                }),
                cx.subscribe(&session, |_, _, change: &Change, cx| {
                    if matches!(change, Change::Snapshot | Change::Connection) {
                        cx.notify();
                    }
                }),
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Submit => this.submit(window, cx),
                        InputEvent::Cancel => this.back(window, cx),
                        InputEvent::Changed => cx.notify(),
                    }
                }),
            ];
        // What Finish keeps listens from the start, so a check started with
        // Finish closed is not missed.
        finishing(&session, cx);
        session.update(cx, |session, _| {
            session.send(json!({"type": "collab_status"}))
        });
        Self {
            session,
            status: None,
            stage: None,
            failure: None,
            report: None,
            external: None,
            menu: Menu::Closed,
            menus_opened: 0,
            input,
            focus_handle: cx.focus_handle(),
            previous_focus: None,
            busy_since: Instant::now(),
            asked: false,
            _subscriptions: subscriptions,
        }
    }

    fn apply(&mut self, event: &Value, cx: &mut Context<Self>) {
        match event["type"].as_str().unwrap_or_default() {
            "collab" => {
                let status: GitStatus =
                    serde_json::from_value(event["status"].clone()).unwrap_or_default();
                let was_busy = self.busy();
                if !status.busy {
                    self.stage = None;
                }
                self.status = Some(status);
                if self.busy() && !was_busy {
                    self.busy_since = Instant::now();
                }
            }
            "collab_sync" => match event["stage"].as_str().unwrap_or_default() {
                "done" => {
                    self.stage = None;
                    self.failure = None;
                    let summary = summary(&event["report"]);
                    if std::mem::take(&mut self.asked) {
                        self.notify(&summary, false, cx);
                    }
                    self.report = Some(summary);
                }
                "failed" => {
                    self.stage = None;
                    let message = event["message"]
                        .as_str()
                        .unwrap_or("The sync failed")
                        .to_owned();
                    // Live sync says a failure once; a person asking hears it again.
                    if std::mem::take(&mut self.asked)
                        || self.failure.as_deref() != Some(message.as_str())
                    {
                        self.notify(&format!("Sync failed: {message}"), true, cx);
                    }
                    self.failure = Some(message);
                }
                stage => {
                    if self.stage.is_none() {
                        self.busy_since = Instant::now();
                    }
                    self.stage = Some(stage.to_owned());
                }
            },
            "collab_pull_request" => {
                if let Some(url) = event["url"].as_str().filter(|url| !url.is_empty()) {
                    cx.open_url(url);
                }
                let created = event["created"].as_bool().unwrap_or(false);
                let words = if created {
                    "Opened a pull request"
                } else {
                    "The pull request is open on GitHub"
                };
                self.notify(words, false, cx);
            }
            "media_status" => {
                self.external = event["external"].as_array().map(Vec::len);
            }
            _ => return,
        }
        cx.notify();
    }

    fn busy(&self) -> bool {
        self.stage.is_some() || self.status.as_ref().is_some_and(|status| status.busy)
    }

    /// Says `words` at the foot of the window for a while.
    fn notify(&self, words: &str, error: bool, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.state.notify(words, error);
            cx.notify();
        });
    }

    /// Sends one collaboration command a person asked for.
    fn send(&mut self, command: Value, cx: &mut Context<Self>) {
        if self.session.update(cx, |session, _| session.send(command)) {
            self.asked = true;
            self.busy_since = Instant::now();
        }
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu != Menu::Closed {
            self.close(window, cx);
            return;
        }
        self.menu = Menu::Open;
        self.menus_opened += 1;
        // It takes the keys, so Esc is its own and not the window's.
        self.previous_focus = window.focused(cx);
        window.focus(&self.focus_handle);
        // What it says is as git has it now.
        self.session.update(cx, |session, _| {
            session.send(json!({"type": "collab_status"}))
        });
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = Menu::Closed;
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous);
        }
        cx.notify();
    }

    /// From words asked for, back to the menu; from the menu, closed.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.menu, Menu::Prompt(_)) {
            self.menu = Menu::Open;
            window.focus(&self.focus_handle);
            cx.notify();
        } else {
            self.close(window, cx);
        }
    }

    fn ask(&mut self, prompt: Prompt, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = Menu::Prompt(prompt);
        self.input.update(cx, |input, cx| {
            input.clear(cx);
            input.set_placeholder(prompt.placeholder(), cx);
        });
        self.input.read(cx).focus(window);
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Menu::Prompt(prompt) = self.menu else {
            return;
        };
        let words = self.input.read(cx).text().trim().to_owned();
        let command = match prompt {
            Prompt::Publish => {
                let mut github = json!({"private": true});
                if !words.is_empty() {
                    github["name"] = json!(words);
                }
                json!({"type": "collab_publish", "github": github})
            }
            Prompt::Commit => json!({"type": "collab_commit", "message": words}),
            Prompt::NewBranch => {
                let name = branch_name(&words);
                if name.is_empty() {
                    return;
                }
                json!({"type": "collab_branch", "name": name, "create": true})
            }
            Prompt::PullRequest => json!({"type": "collab_pull_request", "title": words}),
        };
        self.send(command, cx);
        self.close(window, cx);
    }

    /// Runs `command` from the menu and closes it.
    fn run(&mut self, command: Value, window: &mut Window, cx: &mut Context<Self>) {
        self.send(command, cx);
        self.close(window, cx);
    }

    // ---------------------------------------------------------- drawing

    fn chip(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let status = self.status.clone().unwrap_or_default();
        let busy = self.busy();
        let open = self.menu != Menu::Closed;
        let conflicted = status.conflicted;
        let ink = if conflicted {
            theme::neg()
        } else {
            theme::ink2()
        };
        let mut chip = theme::type_style(div(), text::CONTROL)
            .id("collab-chip")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(theme::space::S6))
            .h(px(height::H24))
            .px(px(theme::space::S8))
            .rounded(px(radius::CONTROL))
            .border_1()
            .border_color(theme::line())
            .cursor_pointer()
            .text_color(ink)
            .when(open, |chip| chip.bg(theme::on()))
            .when(!open, |chip| chip.hover(|style| style.bg(theme::raised())))
            .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx)));
        if busy {
            chip = chip
                .child(spinner(self.busy_since, theme::activity::kerf(), window))
                .child(stage_words(self.stage.as_deref()));
        } else if !status.repository {
            chip = chip.child(branch_glyph(theme::ink3())).child(ui::label(
                "Not shared",
                text::CONTROL,
                theme::ink2(),
            ));
        } else {
            // Live sync's glyph is sage: connected.
            let glyph = if status.live { theme::pos() } else { ink };
            chip = chip
                .child(branch_glyph(glyph))
                .child(
                    ui::line(
                        status.branch.clone().unwrap_or_else(|| "detached".into()),
                        text::CONTROL,
                        ink,
                    )
                    .max_w(px(140.0)),
                )
                .when(status.ahead > 0, |chip| {
                    chip.child(ui::label(format!("↑{}", status.ahead), text::CONTROL, ink))
                })
                .when(status.behind > 0, |chip| {
                    chip.child(ui::label(format!("↓{}", status.behind), text::CONTROL, ink))
                })
                .when(conflicted, |chip| {
                    chip.child(ui::label("Merge unfinished", text::CONTROL, theme::neg()))
                });
        }
        let mark = if self.failure.is_some() && !busy {
            Some(theme::neg())
        } else if status.changed && !busy && !conflicted {
            Some(theme::ink2())
        } else {
            None
        };
        let tooltip: SharedString = match (&self.failure, status.changed) {
            (Some(failure), _) => format!("Sync failed: {failure}").into(),
            _ if conflicted => "A merge is unfinished; finish it with git".into(),
            (None, true) => "Edits not committed yet".into(),
            _ => "Sync".into(),
        };
        // Its menu says all the tooltip would, so the tooltip waits while it is open.
        chip.children(mark.map(|color| ui::dot(color, 6.0)))
            .when(!open, |chip| {
                chip.tooltip(move |_, cx| cx.new(|_| ui::Tooltip(tooltip.clone())).into())
            })
    }

    fn menu(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let body = match self.menu {
            Menu::Prompt(prompt) => self.prompt(prompt, window, cx),
            _ => self.choices(cx),
        };
        div()
            .key_context(MENU_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, window, cx| this.close(window, cx)))
            // A press anywhere else puts it away, and does nothing more: a
            // press on the chip closes it rather than opening it again.
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .occlude()
            .w(px(MENU_WIDTH))
            .flex()
            .flex_col()
            .p(px(theme::space::S4))
            .rounded(px(radius::CARD))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::line2())
            .shadow_lg()
            .font(theme::ui_font(FontWeight::NORMAL))
            .text_color(theme::ink())
            .child(body)
            .with_animation(
                ("collab-menu", self.menus_opened),
                motion::animation(scene::FADE),
                |menu, t| menu.opacity(t),
            )
            .into_any_element()
    }

    /// The menu's head: where the project stands, and what went wrong.
    fn head(&self, status: &GitStatus, cx: &Context<Self>) -> impl IntoElement {
        let revision = self.session.read(cx).revision();
        let (title, standing) = if status.repository {
            let mut parts = vec![format!("Revision {revision}")];
            if status.behind > 0 {
                parts.push(plural(
                    status.behind as usize,
                    "1 commit to pull",
                    "{} commits to pull",
                ));
            }
            if status.ahead > 0 {
                parts.push(plural(
                    status.ahead as usize,
                    "1 commit to push",
                    "{} commits to push",
                ));
            }
            if status.behind == 0 && status.ahead == 0 {
                parts.push(if status.remote.is_some() {
                    "up to date".into()
                } else {
                    "on this computer only".into()
                });
            }
            (
                status.branch.clone().unwrap_or_else(|| "Detached".into()),
                parts.join(" · "),
            )
        } else {
            ("Not shared yet".to_owned(), format!("Revision {revision}"))
        };
        let problem = if status.conflicted {
            Some("A merge is unfinished; finish it with git".to_owned())
        } else if let Some(enclosing) = status.enclosing.as_ref().filter(|_| !status.repository) {
            Some(format!("Inside the repository at {enclosing}; publish it as its own repository to share it"))
        } else {
            self.failure
                .as_ref()
                .map(|failure| format!("Sync failed: {failure}"))
        };
        let note = if self.busy() {
            Some(stage_words(self.stage.as_deref()).to_owned())
        } else {
            self.report.clone()
        };
        let who = status
            .user
            .as_ref()
            .map(|user| format!("Commits as {}", user.name))
            .filter(|_| status.repository);
        div()
            .px(px(theme::space::S8))
            .pt(px(theme::space::S6))
            .pb(px(theme::space::S8))
            .flex()
            .flex_col()
            .gap(px(theme::space::S2))
            .child(ui::line(title, text::TITLE, theme::ink()))
            .child(ui::label(standing, text::META, theme::ink3()))
            .when_some(who, |head, who| {
                head.child(ui::line(who, text::META, theme::ink3()))
            })
            .when_some(problem.clone(), |head, problem| {
                head.child(
                    div()
                        .pt(px(theme::space::S4))
                        .flex()
                        .items_start()
                        .gap(px(theme::space::S6))
                        .child(
                            div()
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .child(ui::dot(theme::neg(), 6.0)),
                        )
                        .child(
                            ui::label(problem, text::META, theme::ink2())
                                .flex_1()
                                .min_w_0(),
                        ),
                )
            })
            .when_some(note.filter(|_| problem.is_none()), |head, note| {
                head.child(ui::label(note, text::META, theme::ink2()).pt(px(theme::space::S2)))
            })
    }

    fn choices(&self, cx: &Context<Self>) -> AnyElement {
        let status = self.status.clone().unwrap_or_default();
        let busy = self.busy();
        let idle = if busy {
            Err("A sync is running")
        } else if status.conflicted {
            Err("Finish the merge in progress with git first")
        } else {
            Ok(())
        };
        let publish_reason = if busy {
            Err("A sync is running")
        } else if !status.gh {
            Err("Needs the GitHub CLI, signed in (gh auth login)")
        } else {
            Ok(())
        };
        let mut items: Vec<AnyElement> = vec![
            self.head(&status, cx).into_any_element(),
            ui::hairline(true).into_any_element(),
        ];
        if !status.repository {
            items.push(
                item(
                    "collab-publish",
                    "Publish to GitHub…",
                    publish_reason,
                    cx.listener(|this, _, window, cx| this.ask(Prompt::Publish, window, cx)),
                )
                .into_any_element(),
            );
            items.push(
                item(
                    "collab-init",
                    "Make a Repository Here",
                    if busy {
                        Err("A sync is running")
                    } else {
                        Ok(())
                    },
                    cx.listener(|this, _, window, cx| {
                        this.run(json!({"type": "collab_publish"}), window, cx);
                    }),
                )
                .into_any_element(),
            );
        } else {
            let live = status.live;
            items.push(
                item(
                    "collab-sync",
                    "Sync Now",
                    idle,
                    cx.listener(|this, _, window, cx| {
                        this.run(json!({"type": "collab_sync"}), window, cx)
                    }),
                )
                .into_any_element(),
            );
            items.push(
                item(
                    "collab-live",
                    "Live Sync",
                    Ok(()),
                    cx.listener(move |this, _, _, cx| this.set_live(!live, cx)),
                )
                .child(div().flex_1())
                // The row takes the press; the switch shows what it did.
                .child(ui::switch("collab-live-switch", live, |_, _, _| {}))
                .into_any_element(),
            );
            let commit = idle.and(if status.changed {
                Ok(())
            } else {
                Err("Nothing to commit")
            });
            items.push(
                item(
                    "collab-commit",
                    "Commit…",
                    commit,
                    cx.listener(|this, _, window, cx| this.ask(Prompt::Commit, window, cx)),
                )
                .into_any_element(),
            );
            items.push(
                ui::hairline(true)
                    .my(px(theme::space::S4))
                    .into_any_element(),
            );
            let current = status.branch.clone();
            let others = status
                .branches
                .iter()
                .any(|branch| Some(branch) != current.as_ref());
            let switch = idle.and(if others {
                Ok(())
            } else {
                Err("There are no other branches")
            });
            let unfolded = self.menu == Menu::Branches;
            items.push(
                item(
                    "collab-switch",
                    "Switch Branch",
                    switch,
                    cx.listener(move |this, _, _, cx| {
                        this.menu = if unfolded { Menu::Open } else { Menu::Branches };
                        cx.notify();
                    }),
                )
                .child(div().flex_1())
                .child(
                    if unfolded {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    }
                    .view(
                        14.0,
                        if switch.is_ok() {
                            theme::ink2()
                        } else {
                            theme::dis()
                        },
                    ),
                )
                .into_any_element(),
            );
            if unfolded && switch.is_ok() {
                items.push(self.branches(&status, cx));
            }
            items.push(
                item(
                    "collab-new-branch",
                    "New Branch…",
                    idle,
                    cx.listener(|this, _, window, cx| this.ask(Prompt::NewBranch, window, cx)),
                )
                .into_any_element(),
            );
            let pull = idle.and(if status.remote.is_some() {
                Ok(())
            } else {
                Err("Publish the project to GitHub first")
            });
            items.push(
                item(
                    "collab-pull-request",
                    "Pull Request…",
                    pull,
                    cx.listener(|this, _, window, cx| this.ask(Prompt::PullRequest, window, cx)),
                )
                .into_any_element(),
            );
            if status.remote.is_none() {
                items.push(
                    item(
                        "collab-publish",
                        "Publish to GitHub…",
                        publish_reason,
                        cx.listener(|this, _, window, cx| this.ask(Prompt::Publish, window, cx)),
                    )
                    .into_any_element(),
                );
            }
        }
        items.push(
            ui::hairline(true)
                .my(px(theme::space::S4))
                .into_any_element(),
        );
        let consolidate = if busy {
            Err("A sync is running")
        } else if self.external == Some(0) {
            Err("All media is inside the project folder")
        } else {
            Ok(())
        };
        items.push(
            item(
                "collab-consolidate",
                "Consolidate Media",
                consolidate,
                cx.listener(|this, _, window, cx| {
                    this.run(json!({"type": "collab_consolidate"}), window, cx)
                }),
            )
            .into_any_element(),
        );
        items.push(
            item(
                "collab-refresh",
                "Refresh",
                Ok(()),
                cx.listener(|this, _, window, cx| {
                    this.session.update(cx, |session, _| {
                        session.send(json!({"type": "collab_status"}))
                    });
                    this.close(window, cx);
                }),
            )
            .into_any_element(),
        );
        div().flex().flex_col().children(items).into_any_element()
    }

    /// The branches, this one checked; another switches to it.
    fn branches(&self, status: &GitStatus, cx: &Context<Self>) -> AnyElement {
        let current = status.branch.clone();
        let mut names: Vec<String> = current.iter().cloned().collect();
        names.extend(
            status
                .branches
                .iter()
                .filter(|name| Some(*name) != current.as_ref())
                .cloned(),
        );
        div()
            .id("collab-branches")
            .max_h(px(200.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(names.into_iter().enumerate().map(|(index, name)| {
                let here = Some(&name) == current.as_ref();
                let row = theme::type_style(div(), text::MENU_QUIET)
                    .id(ElementId::named_usize("collab-branch", index))
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .h(px(28.0))
                    .pl(px(theme::space::S16 + 2.0))
                    .pr(px(10.0))
                    .rounded(px(radius::CONTROL))
                    .text_color(if here { theme::ink() } else { theme::ink2() })
                    .child(if here {
                        Icon::Check.view(14.0, theme::ink2()).into_any_element()
                    } else {
                        crate::icons::spacer(14.0).into_any_element()
                    })
                    .child(
                        ui::line(
                            name.clone(),
                            text::MENU_QUIET,
                            if here { theme::ink() } else { theme::ink2() },
                        )
                        .min_w_0(),
                    );
                if here {
                    return row;
                }
                row.cursor_pointer()
                    .hover(|style| style.bg(theme::popup_hover()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(
                            json!({"type": "collab_branch", "name": name, "create": false}),
                            window,
                            cx,
                        );
                    }))
            }))
            .into_any_element()
    }

    fn set_live(&mut self, on: bool, cx: &mut Context<Self>) {
        // The switch moves at once; the bridge's status follows.
        if let Some(status) = self.status.as_mut() {
            status.live = on;
        }
        self.session
            .update(cx, |session, _| session.set_live_sync(on));
        cx.notify();
    }

    fn prompt(&self, prompt: Prompt, window: &Window, cx: &Context<Self>) -> AnyElement {
        let empty = self.input.read(cx).text().trim().is_empty();
        let ready = !(prompt == Prompt::NewBranch && empty);
        div()
            .p(px(theme::space::S8))
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .child(ui::label(prompt.title(), text::TITLE, theme::ink()))
            .child(ui::label(prompt.says(), text::META, theme::ink3()))
            .child(crate::launcher::field(&self.input, window, cx))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(theme::space::S6))
                    .child(
                        ui::button("collab-prompt-cancel", "Cancel")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child(
                        ui::button("collab-prompt-go", prompt.action())
                            .small()
                            .primary()
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
            .into_any_element()
    }
}

impl Render for Collab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        if !session.has_project() || self.status.is_none() {
            return div().into_any_element();
        }
        let open = self.menu != Menu::Closed;
        div()
            .relative()
            .flex_none()
            .child(self.chip(window, cx))
            .when(open, |wrapper| {
                wrapper.child(
                    // Under the chip, its right edge on the chip's, over everything.
                    div().absolute().top_full().right_0().child(
                        deferred(
                            anchored()
                                .anchor(Corner::TopRight)
                                .offset(point(px(0.0), px(theme::space::S4)))
                                .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                                .child(self.menu(window, cx)),
                        )
                        .with_priority(1),
                    ),
                )
            })
            .into_any_element()
    }
}

// ---------------------------------------------------------- Collab's pieces

/// A row of the chip's menu; `enabled` is Ok, or why it cannot run now.
fn item(
    id: &'static str,
    words: &'static str,
    enabled: Result<(), &'static str>,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let row = theme::type_style(div(), text::MENU_ITEM)
        .id(id)
        .flex()
        .items_center()
        .gap(px(theme::space::S8))
        .h(px(30.0))
        .px(px(10.0))
        .rounded(px(radius::CONTROL))
        .text_color(if enabled.is_ok() {
            theme::ink()
        } else {
            theme::dis()
        })
        .child(words);
    match enabled {
        Ok(()) => row
            .cursor_pointer()
            .hover(|style| style.bg(theme::popup_hover()))
            .on_click(on_click),
        Err(reason) => row
            .tooltip(move |_, cx| cx.new(|_| ui::Tooltip(reason.into())).into())
            // A row that cannot run takes the press, so nothing under it does.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation()),
    }
}

/// What a sync is doing, as a person says it.
fn stage_words(stage: Option<&str>) -> &'static str {
    match stage {
        Some("committing") => "Committing…",
        Some("fetching") => "Pulling…",
        Some("merging") => "Merging…",
        Some("pushing") => "Pushing…",
        Some("downloading") => "Downloading media…",
        _ => "Syncing…",
    }
}

/// What a finished sync did, in a line.
fn summary(report: &Value) -> String {
    let said: Vec<&str> = report["summary"]
        .as_array()
        .map(|lines| {
            lines
                .iter()
                .filter_map(Value::as_str)
                .filter(|line| !line.trim().is_empty())
                .collect()
        })
        .unwrap_or_default();
    if !said.is_empty() {
        return said.join(" · ");
    }
    let mut parts = Vec::new();
    if let Some(branch) = report["branch"].as_str() {
        parts.push(format!("switched to {branch}"));
    }
    if !report["committed"].is_null() {
        parts.push("committed".to_owned());
    }
    let incoming = report["incoming"].as_array().map_or(0, Vec::len);
    if incoming > 0 {
        parts.push(plural(
            incoming,
            "brought in 1 commit",
            "brought in {} commits",
        ));
    }
    let conflicts = report["conflicts"].as_array().map_or(0, Vec::len);
    if conflicts > 0 {
        parts.push(plural(
            conflicts,
            "settled 1 conflict",
            "settled {} conflicts",
        ));
    }
    if report["pushed"].as_bool() == Some(true) {
        parts.push("pushed".to_owned());
    }
    if let Some(remote) = report["remote"].as_str() {
        parts.push(format!("published to {remote}"));
    }
    if parts.is_empty() {
        return "Up to date".to_owned();
    }
    sentence(parts.join(", "))
}

/// A branch name git takes: trimmed, its spaces as dashes.
fn branch_name(words: &str) -> String {
    words.split_whitespace().collect::<Vec<_>>().join("-")
}

/// A branch drawn as a line with a fork off it, in `color`.
fn branch_glyph(color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |area, _, window, _| {
            let scale = f32::from(area.size.width) / 16.0;
            let at = |x: f32, y: f32| {
                point(area.origin.x + px(x * scale), area.origin.y + px(y * scale))
            };
            let mut builder = PathBuilder::stroke(px(1.3));
            let ring = |builder: &mut PathBuilder, x: f32, y: f32| {
                let r = 1.9;
                builder.move_to(at(x + r, y));
                builder.arc_to(
                    point(px(r * scale), px(r * scale)),
                    px(0.0),
                    false,
                    true,
                    at(x - r, y),
                );
                builder.arc_to(
                    point(px(r * scale), px(r * scale)),
                    px(0.0),
                    false,
                    true,
                    at(x + r, y),
                );
            };
            ring(&mut builder, 5.0, 3.5);
            ring(&mut builder, 5.0, 12.5);
            ring(&mut builder, 11.5, 4.5);
            builder.move_to(at(5.0, 5.4));
            builder.line_to(at(5.0, 10.6));
            builder.move_to(at(11.5, 6.4));
            builder.cubic_bezier_to(at(5.0, 10.6), at(11.5, 9.0), at(5.0, 8.0));
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(14.0))
    .flex_none()
}

/// A ring turning once each [`scene::WORK_PULSE`] while work runs; still
/// under Reduce Motion.
fn spinner(since: Instant, color: Rgba, window: &Window) -> impl IntoElement {
    let phase = motion::phase(since, scene::WORK_PULSE);
    if !motion::reduced() {
        window.request_animation_frame();
    }
    canvas(
        |_, _, _| (),
        move |area, _, window, _| {
            let size = f32::from(area.size.width.min(area.size.height));
            let r = size / 2.0 - 1.5;
            let centre = point(
                area.origin.x + px(size / 2.0),
                area.origin.y + px(size / 2.0),
            );
            let at = |angle: f32| {
                point(
                    centre.x + px(r * angle.cos()),
                    centre.y + px(r * angle.sin()),
                )
            };
            let start = phase * std::f32::consts::TAU;
            let end = start + std::f32::consts::PI * 1.5;
            let mut track = PathBuilder::stroke(px(1.5));
            track.move_to(at(0.0));
            track.arc_to(
                point(px(r), px(r)),
                px(0.0),
                false,
                true,
                at(std::f32::consts::PI),
            );
            track.arc_to(point(px(r), px(r)), px(0.0), false, true, at(0.0));
            if let Ok(path) = track.build() {
                window.paint_path(path, theme::track());
            }
            let mut arc = PathBuilder::stroke(px(1.5));
            arc.move_to(at(start));
            arc.arc_to(point(px(r), px(r)), px(0.0), true, true, at(end));
            if let Ok(path) = arc.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(12.0))
    .flex_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_take_a_true_minus_and_no_negative_zero() {
        assert_eq!(number(-23.06, 1), "−23.1");
        assert_eq!(number(-0.01, 1), "0.0");
        assert_eq!(number(4.0, 0), "4");
    }

    #[test]
    fn a_long_series_is_drawn_in_at_most_the_chart_points_and_keeps_silence() {
        let series: Vec<_> = (0..6000)
            .map(|i| {
                let at = i as f32 * 0.1;
                let heard = if i < 3000 { Some(-20.0) } else { None };
                (at, heard, heard)
            })
            .collect();
        let points = chart_points(&series, 600.0);
        assert!(points.len() <= CHART_POINTS);
        assert_eq!(points.first().unwrap().short, Some(-20.0));
        assert_eq!(points.last().unwrap().short, None);
        assert!(points.windows(2).all(|pair| pair[0].x <= pair[1].x));
    }

    #[test]
    fn a_sync_report_reads_as_a_line() {
        assert_eq!(
            summary(&json!({"summary": ["Nothing to commit"]})),
            "Nothing to commit"
        );
        assert_eq!(
            summary(
                &json!({"committed": "abc", "incoming": [{}, {}], "pushed": true, "summary": []})
            ),
            "Committed, brought in 2 commits, pushed"
        );
        assert_eq!(
            summary(&json!({"committed": null, "incoming": [], "pushed": false})),
            "Up to date"
        );
        assert_eq!(
            summary(&json!({"committed": null, "branch": "colour-pass", "previous": "main"})),
            "Switched to colour-pass"
        );
    }

    #[test]
    fn a_check_report_lists_what_blocks_first() {
        let event = json!({
            "status": "done", "valid": false,
            "findings": [
                {"severity": "info", "code": "a", "message": "said"},
                {"severity": "error", "code": "b", "message": "blocks", "at": 2.0, "timecode": "00:00:02:00"},
                {"severity": "warning", "code": "c", "message": "look"},
            ],
            "summary": {"passed": 1, "total": 2, "blocking": 1, "rows": [
                {"code": "x", "name": "True peak", "passed": false, "value": "−0.4 dBTP", "limit": "≤ −1 dBTP", "count": 1, "blocking": 1},
                {"code": "y", "name": "", "passed": true, "value": "0", "count": 0, "blocking": 0},
            ]},
        });
        let report = Report::from_value(&event, 7);
        let order: Vec<_> = report
            .findings
            .iter()
            .map(|finding| finding.message.as_str())
            .collect();
        assert_eq!(order, ["blocks", "look", "said"]);
        assert_eq!(report.summary.rows[1].name(), "y");
        assert_eq!(report.summary.rows[0].color(), theme::neg());
        assert!(!report.valid);
    }

    #[test]
    fn what_the_bridge_says_begins_with_a_capital() {
        assert_eq!(
            sentence("the sequence has no captions".into()),
            "The sequence has no captions"
        );
        assert_eq!(sentence("Already".into()), "Already");
        assert_eq!(sentence(String::new()), "");
    }

    #[test]
    fn branch_names_lose_their_spaces() {
        assert_eq!(branch_name("  colour pass  two "), "colour-pass-two");
    }
}
