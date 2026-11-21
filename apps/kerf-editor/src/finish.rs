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

/// The panel at its largest; the workspace places it ([`crate::ui::modal_bounds`])
/// and rounds its corners, and it fills that.
pub const MOST: (f32, f32) = (860.0, 600.0);
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
/// The chip's menu, and its questions that hold more than a line.
const MENU_WIDTH: f32 = 284.0;
const PROMPT_WIDTH: f32 = 380.0;
/// A pull request's description: its least height, and the most it grows to.
const BODY_LEAST: f32 = 72.0;
const BODY_MOST: f32 = 144.0;

// ------------------------------------------------------------ what is kept

/// The last check's report, as `check {status: done}` sent it.
struct Report {
    /// The document revision checked.
    revision: u64,
    valid: bool,
    summary: Summary,
    /// The summary as the bridge sent it, for Export's Preflight.
    summary_value: Value,
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
            summary_value: event["summary"].clone(),
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

// ------------------------------------------------------------------ preflight

/// Fills the conversation's composer with `words`, to be changed before it
/// is sent (Qt's askPrefill): a failed check's Ask Kerf, in Export's
/// Preflight and in Finish's QC. The window's root is to hand it to the
/// conversation; until something on the way to the keys takes it
/// ([`can_ask`]), neither offers Ask Kerf.
#[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct AskKerf {
    pub words: SharedString,
}

/// What Deliver says under its title: where the master and the review copy
/// go in `project`, the master where the project keeps it to go
/// ([`crate::export::master_request`], `kept`): "The master and a review
/// copy go to deliveries/ in the project, numbered when one is there
/// already.", or "The master goes to masters/ and a review copy to
/// deliveries/ in the project, each numbered when one is there already."
pub fn deliver_said(project: Option<&Path>, kept: Option<&crate::prefs::MasterOutput>) -> String {
    /// Where a request sends its file: the folder as people read it, and
    /// whether it is in the project; None where kerfd puts renders.
    fn place(command: &Value) -> Option<(String, bool)> {
        command["output"].as_str().map(|folder| {
            (
                crate::export::shown_folder(folder),
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
    let master = place(&crate::export::master_request(project, kept));
    let review = place(&crate::session::export_request(project, "review"));
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

/// Whether [`AskKerf`] is heard in `window`: something on the way from the
/// keys to the window's root takes it. Export's Preflight and Finish's QC
/// ask as they draw, and offer Ask Kerf only then.
pub fn can_ask(window: &Window, cx: &mut App) -> bool {
    window.is_action_available(&AskKerf::default(), cx)
}

/// How many rows Preflight shows: every failed one up to the most, then
/// passed ones up to the least in all.
pub const PREFLIGHT_LEAST: usize = 4;
pub const PREFLIGHT_MOST: usize = 6;

/// One row of Export's Preflight: a check of the last QC pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreflightRow {
    pub code: String,
    /// Its name and what it found: "True peak · −0.4 dBTP".
    pub label: String,
    pub passed: bool,
    /// What a passed check allows, said at its end.
    pub limit: Option<String>,
    /// What Ask Kerf fills the composer with, for one that failed.
    pub ask: Option<String>,
}

/// The rows Preflight shows of a check's `summary` (`{rows: [{code, name,
/// passed, value, limit}]}`): every failed one up to `most`, then passed
/// ones up to `least` in all, in the summary's order. A row that does not
/// say passed it.
pub fn preflight_rows(summary: &Value, least: usize, most: usize) -> Vec<PreflightRow> {
    let rows = summary["rows"].as_array().cloned().unwrap_or_default();
    let passed = |row: &Value| row["passed"].as_bool().unwrap_or(true);
    let mut chosen: Vec<usize> = (0..rows.len())
        .filter(|index| !passed(&rows[*index]))
        .take(most)
        .collect();
    for index in 0..rows.len() {
        if chosen.len() >= least {
            break;
        }
        if passed(&rows[index]) && !chosen.contains(&index) {
            chosen.push(index);
        }
    }
    chosen.sort_unstable();
    chosen
        .into_iter()
        .map(|index| {
            let row = &rows[index];
            let code = row["code"].as_str().unwrap_or_default().to_owned();
            let name = row["name"].as_str().unwrap_or(&code).to_owned();
            let value = row["value"].as_str().unwrap_or_default();
            let ok = passed(row);
            PreflightRow {
                label: if value.is_empty() {
                    name
                } else {
                    format!("{name} · {value}")
                },
                passed: ok,
                limit: row["limit"]
                    .as_str()
                    .filter(|limit| ok && !limit.is_empty())
                    .map(str::to_owned),
                ask: (!ok).then(|| ask_text(row)),
                code,
            }
        })
        .collect()
}

/// The failed rows Preflight leaves out of `summary`'s.
pub fn preflight_hidden(summary: &Value, most: usize) -> usize {
    let failed = summary["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|row| !row["passed"].as_bool().unwrap_or(true))
                .count()
        })
        .unwrap_or(0);
    failed.saturating_sub(most)
}

/// What Ask Kerf asks of a failed check (`{name, value, limit}`): "Bring
/// true peak to ≤ −1 dBTP (now −0.4 dBTP)", or, with no limit, "Fix the
/// flash (3 frames)".
pub fn ask_text(row: &Value) -> String {
    let name = row["name"]
        .as_str()
        .or_else(|| row["code"].as_str())
        .unwrap_or_default();
    let name = lower_first(name);
    let value = row["value"].as_str().unwrap_or_default();
    let limit = row["limit"].as_str().unwrap_or_default();
    match (limit.is_empty(), value.is_empty()) {
        (false, true) => format!("Bring {name} to {limit}"),
        (false, false) => format!("Bring {name} to {limit} (now {value})"),
        (true, true) => format!("Fix the {name}"),
        (true, false) => format!("Fix the {name} ({value})"),
    }
}

/// `text` with its first letter lower, unless the second is a capital (an
/// initialism: "LUFS").
fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    let (Some(first), Some(second)) = (chars.next(), chars.next()) else {
        return text.to_owned();
    };
    if second.is_uppercase() {
        return text.to_owned();
    }
    first.to_lowercase().chain(text.chars().skip(1)).collect()
}

/// What Export's Preflight shows of the project's last QC pass, as Finish
/// keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preflight {
    /// A check's progress while it runs.
    pub checking: Option<f64>,
    /// Why the last check failed.
    pub failed: Option<String>,
    /// A check has finished since the project opened.
    pub checked: bool,
    pub rows: Vec<PreflightRow>,
    /// Failed checks left out of the rows.
    pub hidden: usize,
}

/// The project's last QC pass for Export's Preflight. Export calls it as
/// it draws, and draws again on each `check` event.
pub fn preflight(session: &Entity<Session>, cx: &mut App) -> Preflight {
    let finishing = finishing(session, cx);
    let finishing = finishing.read(cx);
    let summary = finishing
        .report
        .as_ref()
        .map(|report| &report.summary_value);
    Preflight {
        checking: finishing.checking,
        failed: finishing.check_failed.clone(),
        checked: summary.is_some(),
        rows: summary
            .map(|summary| preflight_rows(summary, PREFLIGHT_LEAST, PREFLIGHT_MOST))
            .unwrap_or_default(),
        hidden: summary.map_or(0, |summary| preflight_hidden(summary, PREFLIGHT_MOST)),
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
    /// Whether the window fills the composer from a failed check's Ask
    /// Kerf ([`can_ask`]), as it was when last drawn.
    can_ask: bool,
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
            can_ask: false,
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

    // ---------------------------------------------------------- QC

    fn qc(&self, cx: &Context<Self>) -> Vec<AnyElement> {
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
                crate::palette::unavailable(&states, "qc.run")
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
                let ask = (!row.passed && self.can_ask).then(|| {
                    let words: SharedString = ask_text(&json!({
                        "name": row.name(),
                        "value": row.value,
                        "limit": row.limit,
                    }))
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
        let kept = folder
            .as_deref()
            .and_then(|project| session.prefs.master_output(project));
        let (master_goes, master_makes) = crate::export::master_goes(folder.as_deref(), kept);
        let heading = deliver_said(folder.as_deref(), kept);
        let mut out = vec![self.title("Deliver", heading, None)];
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.can_ask = can_ask(window, cx);
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
    /// The one the registry's command `id` asks with.
    fn of_command(id: &str) -> Option<Prompt> {
        match id {
            "collab.publish" => Some(Prompt::Publish),
            "collab.commit" => Some(Prompt::Commit),
            "collab.newBranch" => Some(Prompt::NewBranch),
            "collab.pullRequest" => Some(Prompt::PullRequest),
            _ => None,
        }
    }

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
            Prompt::Commit => "Describe the change",
            Prompt::NewBranch => "Name",
            Prompt::PullRequest => "Title",
        }
    }

    /// What it does, said over its fields; `branch` is the one checked out.
    fn says(self, branch: Option<&str>) -> String {
        match self {
            Prompt::Publish => "Makes the project a Git repository if it is not one yet, then pushes it so collaborators can clone it, sync, and open pull requests.".into(),
            Prompt::Commit => "Commits your edits without syncing. Left empty, the message describes them.".into(),
            Prompt::NewBranch => "Commits your edits, then starts a branch from them. The next sync publishes it.".into(),
            Prompt::PullRequest => format!(
                "Pushes {} and opens a pull request on GitHub. Reviewers read the change as Kerf code.",
                branch.filter(|branch| !branch.is_empty()).unwrap_or("this branch")
            ),
        }
    }

    fn action(self) -> &'static str {
        match self {
            Prompt::Publish => "Publish",
            Prompt::Commit => "Commit",
            Prompt::NewBranch => "Create Branch",
            Prompt::PullRequest => "Open Pull Request",
        }
    }

    /// Its panel is wider than the menu: it holds more than a line.
    fn wide(self) -> bool {
        matches!(self, Prompt::Publish | Prompt::PullRequest)
    }
}

/// How Publish shares the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishWay {
    /// A new repository on GitHub, made with the GitHub CLI.
    GitHub,
    /// A remote the person already has (GitHub, GitLab, their own server).
    Remote,
    /// A repository on this computer only.
    Local,
}

impl PublishWay {
    pub const ALL: [PublishWay; 3] = [PublishWay::GitHub, PublishWay::Remote, PublishWay::Local];

    pub fn label(self) -> &'static str {
        match self {
            PublishWay::GitHub => "GitHub",
            PublishWay::Remote => "Remote",
            PublishWay::Local => "This computer",
        }
    }

    /// What choosing it does, in the Qt editor's words.
    pub fn says(self) -> &'static str {
        match self {
            PublishWay::GitHub => "Create a GitHub repository",
            PublishWay::Remote => "Push to a remote I already have",
            PublishWay::Local => "Only make it a repository on this computer",
        }
    }
}

/// Why GitHub cannot be chosen without its command-line tool.
pub const NEEDS_GH: &str = "Install the GitHub CLI and sign in with `gh auth login`";

/// The repository name Publish starts with: the project's folder name,
/// each run of anything but letters, digits, `.`, `_` and `-` one `-`.
pub fn publish_name(folder: &str) -> String {
    let mut name = String::new();
    let mut gap = false;
    for ch in folder.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            if gap {
                name.push('-');
            }
            gap = false;
            name.push(ch);
        } else {
            gap = true;
        }
    }
    if gap {
        name.push('-');
    }
    name
}

/// The title Open Pull Request starts with: the branch's name, its dashes
/// and underscores as spaces, its first letter a capital.
pub fn pull_request_title(branch: &str) -> String {
    let words = branch.replace(['-', '_'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What Publish sends for `way`: a GitHub repository named `name` (the
/// folder's when empty), private or not; a push to `remote`; or a
/// repository here only. None while the remote is not given.
pub fn publish_command(way: PublishWay, name: &str, private: bool, remote: &str) -> Option<Value> {
    match way {
        PublishWay::GitHub => {
            let mut github = json!({"private": private});
            if !name.trim().is_empty() {
                github["name"] = json!(name.trim());
            }
            Some(json!({"type": "collab_publish", "github": github}))
        }
        PublishWay::Remote => {
            let remote = remote.trim();
            (!remote.is_empty()).then(|| json!({"type": "collab_publish", "remote": remote}))
        }
        PublishWay::Local => Some(json!({"type": "collab_publish"})),
    }
}

/// What Open Pull Request sends: its title, and its description when one
/// is written (left out, the bridge lists the branch's commits). None
/// without a title.
pub fn pull_request_command(title: &str, body: &str) -> Option<Value> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    let mut command = json!({"type": "collab_pull_request", "title": title});
    if !body.trim().is_empty() {
        command["body"] = json!(body);
    }
    Some(command)
}

/// What the window says of a pull request the bridge answered for
/// (`collab_pull_request {url, created}`): one opened; GitHub's page for
/// starting one (without the GitHub CLI the bridge opens a `/compare` page);
/// or the one the branch has already.
pub fn pull_request_said(url: &str, created: bool) -> String {
    let compare = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .split('/')
        .nth(3)
        == Some("compare");
    let words = if created {
        "Opened a pull request"
    } else if compare {
        "Opened GitHub's page for a new pull request"
    } else {
        "This branch already has a pull request"
    };
    if url.trim().is_empty() {
        words.to_owned()
    } else {
        format!("{words}: {url}")
    }
}

/// The lines a finished sync's `report` leaves in Activity: each commit it
/// brought in ("jun · Trimmed c3"), each conflict it settled ("Kept yours ·
/// project.kerf: what"), each clip renumbered ("Renumbered · c4 → c7").
pub fn sync_lines(report: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    for commit in report["incoming"].as_array().into_iter().flatten() {
        lines.push(format!(
            "{} · {}",
            commit["author"].as_str().unwrap_or_default(),
            commit["subject"].as_str().unwrap_or_default()
        ));
    }
    for conflict in report["conflicts"].as_array().into_iter().flatten() {
        let kept = if conflict["kept"] == "theirs" {
            "Kept theirs"
        } else {
            "Kept yours"
        };
        let path = conflict["path"].as_str().unwrap_or_default();
        let what = conflict["what"].as_str().unwrap_or_default();
        let said = if path.is_empty() {
            what.to_owned()
        } else {
            format!("{path}: {what}")
        };
        lines.push(format!("{kept} · {said}"));
    }
    for pair in report["renamed"].as_array().into_iter().flatten() {
        let side = |index: usize| pair[index].as_str().unwrap_or_default().to_owned();
        lines.push(format!("Renumbered · {} → {}", side(0), side(1)));
    }
    lines
}

/// What the window says of a finished sync: "Synced: 2 commits in, pushed,
/// 1 change kept yours" whenever it brought anything (a Live Sync cycle
/// too), "Synced: up to date" when it brought nothing but was `asked` for;
/// None for a quiet Live Sync cycle.
pub fn sync_said(report: &Value, asked: bool) -> Option<String> {
    let incoming = report["incoming"].as_array().map_or(0, Vec::len);
    let conflicts = report["conflicts"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let theirs = conflicts
        .iter()
        .filter(|conflict| conflict["kept"] == "theirs")
        .count();
    let yours = conflicts.len() - theirs;
    let mut parts = Vec::new();
    if incoming > 0 {
        parts.push(plural(incoming, "1 commit in", "{} commits in"));
    }
    if report["pushed"].as_bool() == Some(true) {
        parts.push("pushed".to_owned());
    }
    if yours > 0 {
        parts.push(plural(
            yours,
            "1 change kept yours",
            "{} changes kept yours",
        ));
    }
    if theirs > 0 {
        parts.push(plural(
            theirs,
            "1 change kept theirs",
            "{} changes kept theirs",
        ));
    }
    if parts.is_empty() {
        asked.then(|| "Synced: up to date".to_owned())
    } else {
        Some(format!("Synced: {}", parts.join(", ")))
    }
}

/// A sync's failure as the window keeps it (Qt's `m_syncFailure`): why the
/// last sync failed, until one succeeds (the chip's dot and tip, its menu,
/// View ▸ Sync), and whether it stands, no sync having started since (the
/// window's error bar and the toolbar's red line). What it last said is
/// remembered too: Live Sync says a failure that repeats only once; one a
/// person asked for is said each time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncFailure {
    last: Option<String>,
    standing: bool,
}

impl SyncFailure {
    /// A sync started (a stage of it came): the failure stands no longer.
    pub fn started(&mut self) {
        self.standing = false;
    }

    /// A sync failed, saying `message`; `asked` when a person asked for it.
    /// Whether it is to be said: always when asked, else only when it is
    /// not the failure said last.
    pub fn failed(&mut self, message: &str, asked: bool) -> bool {
        let said = asked || self.last.as_deref() != Some(message);
        self.last = Some(message.to_owned());
        self.standing = true;
        said
    }

    /// A sync succeeded: no failure is kept.
    pub fn done(&mut self) {
        self.last = None;
        self.standing = false;
    }

    /// Why the last sync failed, until one succeeds.
    pub fn last(&self) -> Option<&str> {
        self.last.as_deref()
    }

    /// Why the last sync failed, while no sync has started since.
    pub fn standing(&self) -> Option<&str> {
        self.last.as_deref().filter(|_| self.standing)
    }
}

/// What the Git chip tells the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollabEvent {
    /// A sync failed: why, and whether it is to be said (the window's
    /// error bar, "Sync failed: <why>"), by [`SyncFailure::failed`].
    SyncFailed { why: String, said: bool },
    /// A sync started or succeeded: the failure stands no longer.
    SyncCleared,
}

/// The project's standing against its Git repository, in the toolbar, with
/// Sync and the rest of sharing in its menu. The window's menus and the
/// palette reach its questions too ([`Collab::open_command`]).
pub struct Collab {
    session: Entity<Session>,
    status: Option<GitStatus>,
    /// The step a sync is at (committing, fetching…) while it runs.
    stage: Option<String>,
    /// Why the last sync failed, until one succeeds, and whether it stands.
    failure: SyncFailure,
    /// What the last sync did.
    report: Option<String>,
    menu: Menu,
    /// The kit's menu of its rows, while it shows them (not a question).
    kit: Option<Entity<ui::Menu>>,
    /// How many times the menu has opened, so each opening fades in.
    menus_opened: u64,
    /// Itself, for what its menu's rows do.
    this: gpui::WeakEntity<Self>,
    /// A question's line: the message, the name, the title.
    input: Entity<TextInput>,
    /// Publish's remote.
    remote: Entity<TextInput>,
    /// A pull request's description.
    body: Entity<TextInput>,
    publish_way: PublishWay,
    publish_private: bool,
    focus_handle: FocusHandle,
    /// The chip's own Tab stop (between the format bank and Undo).
    chip_focus: FocusHandle,
    /// Enter or Space went down on the chip: as it comes up it opens the
    /// menu. An Enter that chose in the menu, coming up once the keys are
    /// back on the chip, does not.
    chip_armed: bool,
    /// What had the keys before the menu took them, to give them back.
    previous_focus: Option<FocusHandle>,
    /// Whether the question open was asked from a row of the menu, in its
    /// place: Esc and Cancel go back to the menu. One asked from the
    /// window's menus or the palette (Qt's dialogs) just closes.
    asked_over_menu: bool,
    /// When the work under way began, for the spinner.
    busy_since: Instant,
    /// The command a person asked for (its bridge type), so its end is said.
    asked: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Collab {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_close(cx);
        let input = cx.new(|cx| TextInput::new(cx, ""));
        let remote = cx.new(|cx| TextInput::new(cx, "git@github.com:you/film.git"));
        let body = cx.new(|cx| {
            TextInput::new(cx, "Leave empty to list this branch's commits")
                .multiline()
                .max_height(px(BODY_MOST))
        });
        let mut subscriptions = vec![
            cx.subscribe(&session, |this, _, event: &BridgeEvent, cx| {
                this.apply(&event.0, cx)
            }),
            cx.subscribe(&session, |this, _, change: &Change, cx| {
                if matches!(change, Change::Snapshot | Change::Connection) {
                    // What the menu's rows can do follows the window's commands.
                    this.refresh_menu(cx);
                    cx.notify();
                }
            }),
        ];
        for field in [&input, &remote, &body] {
            subscriptions.push(cx.subscribe_in(
                field,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Submit => this.submit(window, cx),
                    InputEvent::Cancel => this.back(window, cx),
                    InputEvent::Changed => cx.notify(),
                },
            ));
        }
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
            failure: SyncFailure::default(),
            report: None,
            menu: Menu::Closed,
            kit: None,
            menus_opened: 0,
            this: cx.weak_entity(),
            input,
            remote,
            body,
            publish_way: PublishWay::GitHub,
            publish_private: true,
            focus_handle: cx.focus_handle(),
            chip_focus: cx.focus_handle().tab_stop(true),
            chip_armed: false,
            previous_focus: None,
            asked_over_menu: false,
            busy_since: Instant::now(),
            asked: None,
            _subscriptions: subscriptions,
        }
    }

    /// Carries out the registry's collaboration command `id` from the
    /// window's menus or the palette, as the chip's menu does: Commit…, New
    /// Branch…, Open Pull Request… and Publish to GitHub… open the menu on
    /// their question; Switch Branch opens it on the branches. False for a
    /// command it does not ask for, or while the project's Git standing is
    /// not known yet. The workspace calls it (its `run_command`).
    pub fn open_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.status.is_none() || !self.session.read(cx).has_project() {
            return false;
        }
        let prompt = Prompt::of_command(id);
        if prompt.is_none() && id != "collab.switchBranch" {
            return false;
        }
        // What asked (the palette) gives the keys back as it goes, after
        // this: the menu or its question opens once it has, and takes them.
        cx.defer_in(window, move |this, window, cx| {
            if this.menu == Menu::Closed {
                this.menus_opened += 1;
                this.previous_focus = window.focused(cx);
                this.session.update(cx, |session, _| {
                    session.send(json!({"type": "collab_status"}))
                });
            }
            // The menu open gives the question, or itself opened anew, its place.
            if let Some(kit) = this.kit.take() {
                kit.update(cx, |menu, cx| menu.close(window, cx));
            }
            match prompt {
                Some(prompt) => this.ask(prompt, false, window, cx),
                None => this.show_menu(true, window, cx),
            }
        });
        true
    }

    /// Sync Now from the window's menus or the palette: as the chip's, its
    /// end is said. The workspace calls it for `collab.sync`.
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        self.send(json!({"type": "collab_sync"}), cx);
        cx.notify();
    }

    /// Why the last sync failed, until one succeeds (the app's menu's
    /// View ▸ Sync says it, as the chip's own menu does).
    pub fn failure(&self) -> Option<&str> {
        self.failure.last()
    }

    /// Why the last sync failed, while no sync has started since: the
    /// toolbar's red line says it (Qt's standingProblem).
    pub fn standing_failure(&self) -> Option<&str> {
        self.failure.standing()
    }

    /// Whether a merge is unfinished, to be finished with git.
    pub fn conflicted(&self) -> bool {
        self.status.as_ref().is_some_and(|status| status.conflicted)
    }

    /// Whether one of its questions (Commit…, New Branch…, Pull Request…,
    /// Publish to GitHub…) shows, its keys in it or not: Qt asked these in
    /// modal dialogs, so the window's Esc leaves them be and their own
    /// Close has it (`Workspace`'s Esc intercept).
    pub fn asking(&self, cx: &App) -> bool {
        matches!(self.menu, Menu::Prompt(_))
            && self.status.is_some()
            && self.session.read(cx).has_project()
    }

    /// Live Sync from the window's menus or the palette: turned the other
    /// way and remembered for the project. The workspace calls it for
    /// `collab.live`.
    pub fn toggle_live(&mut self, cx: &mut Context<Self>) {
        let live = self.status.as_ref().is_some_and(|status| status.live);
        self.set_live(!live, cx);
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
                    if self.failure.last().is_some() {
                        cx.emit(CollabEvent::SyncCleared);
                    }
                    self.failure.done();
                    let report = &event["report"];
                    let lines = sync_lines(report);
                    if !lines.is_empty() {
                        self.session.update(cx, |session, _| {
                            for line in &lines {
                                session.activity.push("sync", line);
                            }
                        });
                    }
                    // A sync is said as the Qt editor said it; what else a
                    // person asked for (a commit, a branch) as it went.
                    let said = match self.asked.take().as_deref() {
                        Some("collab_sync") => sync_said(report, true),
                        Some(_) => Some(summary(report)),
                        None => sync_said(report, false),
                    };
                    // Held as Qt held it (main_window.cpp's 8 s).
                    if let Some(said) = said {
                        self.notify(&said, false, Some(scene::NOTICE_LONG_HOLD), cx);
                    }
                    self.report = Some(summary(report));
                }
                "failed" => {
                    self.stage = None;
                    let message = event["message"]
                        .as_str()
                        .unwrap_or("The sync did not finish.")
                        .to_owned();
                    // Live sync says a failure once; a person asking hears it
                    // again. It is said in the window's error bar, which
                    // stays until it is dismissed (Qt's showError).
                    let asked = self.asked.take().is_some();
                    let said = self.failure.failed(&message, asked);
                    if said {
                        self.session
                            .update(cx, |session, _| session.activity.push("sync", &message));
                    }
                    cx.emit(CollabEvent::SyncFailed { why: message, said });
                }
                stage => {
                    if self.stage.is_none() {
                        self.busy_since = Instant::now();
                    }
                    self.stage = Some(stage.to_owned());
                    if self.failure.standing().is_some() {
                        cx.emit(CollabEvent::SyncCleared);
                    }
                    self.failure.started();
                }
            },
            "collab_pull_request" => {
                self.asked = None;
                let url = event["url"].as_str().unwrap_or_default().trim().to_owned();
                let created = event["created"].as_bool().unwrap_or(false);
                // Held as Qt held it (main_window.cpp's 8 s).
                self.notify(
                    &pull_request_said(&url, created),
                    false,
                    Some(scene::NOTICE_LONG_HOLD),
                    cx,
                );
                if !url.is_empty() {
                    self.session
                        .update(cx, |session, _| session.activity.push("sync", &url));
                    cx.open_url(&url);
                }
            }
            _ => return,
        }
        // The menu open says what git says now.
        self.refresh_menu(cx);
        cx.notify();
    }

    fn busy(&self) -> bool {
        self.stage.is_some() || self.status.as_ref().is_some_and(|status| status.busy)
    }

    /// Says `words` at the foot of the window for a while: `hold`, else
    /// as long as a notice (or an error) stays.
    fn notify(
        &self,
        words: &str,
        error: bool,
        hold: Option<std::time::Duration>,
        cx: &mut Context<Self>,
    ) {
        self.session.update(cx, |session, cx| {
            session.state.notify_held(words, error, hold);
            cx.emit(Change::Notice);
            cx.notify();
        });
    }

    /// Sends one collaboration command a person asked for.
    fn send(&mut self, command: Value, cx: &mut Context<Self>) {
        let kind = command["type"].as_str().map(str::to_owned);
        if self.session.update(cx, |session, _| session.send(command)) {
            self.asked = kind;
            self.busy_since = Instant::now();
        }
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu != Menu::Closed {
            self.close(window, cx);
        } else {
            self.open(window, cx);
        }
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menus_opened += 1;
        // The keys go back here once it goes.
        self.previous_focus = window.focused(cx);
        // What it says is as git has it now.
        self.session.update(cx, |session, _| {
            session.send(json!({"type": "collab_status"}))
        });
        self.show_menu(false, window, cx);
    }

    /// The menu's rows, the kit's: it takes the keys (the arrows, Return,
    /// a letter, Esc are its own) and gives them back to what had them
    /// before it opened. `branches`: opened on Switch Branch's submenu.
    fn show_menu(&mut self, branches: bool, window: &mut Window, cx: &mut Context<Self>) {
        // From a question, the keys go back where they were first.
        if let Some(previous) = self.previous_focus.clone() {
            window.focus(&previous);
        }
        let items = self.items(cx);
        let switch_at = items
            .iter()
            .position(|item| item.words().as_ref() == "Switch Branch");
        let head_of = cx.entity().downgrade();
        let kit = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx)
                .width(MENU_WIDTH)
                .head(move |_, cx| match head_of.upgrade() {
                    Some(collab) => collab.read(cx).head_and_line(cx),
                    None => div().into_any_element(),
                });
            match switch_at.filter(|_| branches) {
                Some(at) => menu.opened(at),
                None => menu,
            }
        });
        cx.subscribe_in(&kit, window, |this, kit, _: &DismissEvent, _, cx| {
            if this.kit.as_ref() == Some(kit) {
                this.kit = None;
                // A row that asks for words has the menu's place.
                if matches!(this.menu, Menu::Open | Menu::Branches) {
                    this.menu = Menu::Closed;
                }
                cx.notify();
            }
        })
        .detach();
        self.kit = Some(kit);
        self.menu = if branches { Menu::Branches } else { Menu::Open };
        cx.notify();
    }

    /// The rows of the menu open as things stand now (git answered, a sync
    /// began or ended, the window's commands changed).
    fn refresh_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(kit) = self.kit.clone() {
            let items = self.items(cx);
            kit.update(cx, |menu, cx| menu.set_items(items, cx));
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = Menu::Closed;
        // The kit's menu may be what chose this: it goes once that is done.
        if let Some(kit) = self.kit.take() {
            cx.defer_in(window, move |_, window, cx| {
                kit.update(cx, |menu, cx| menu.close(window, cx));
            });
        }
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous);
        }
        cx.notify();
    }

    /// From words asked for in the menu's place, back to the menu; from
    /// the menu, or words asked for from the window's menus or the
    /// palette, closed.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.menu, Menu::Prompt(_)) && self.asked_over_menu {
            self.show_menu(false, window, cx);
        } else {
            self.close(window, cx);
        }
    }

    /// Asks for the words `prompt` needs, in the menu's place: from one of
    /// its rows (`over_menu`), or from the window's menus or the palette.
    fn ask(
        &mut self,
        prompt: Prompt,
        over_menu: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = Menu::Prompt(prompt);
        self.asked_over_menu = over_menu;
        let status = self.status.clone().unwrap_or_default();
        let start = match prompt {
            Prompt::Publish => {
                // The folder's name, as the Qt editor took it; one with no
                // letter GitHub takes (a Hangul name) gives the project's title.
                let session = self.session.read(cx);
                let folder = session
                    .project
                    .as_deref()
                    .and_then(std::path::Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                [folder, session.project_name()]
                    .iter()
                    .map(|name| publish_name(name))
                    .find(|name| name.chars().any(|ch| ch.is_ascii_alphanumeric()))
                    .unwrap_or_default()
            }
            Prompt::PullRequest => pull_request_title(status.branch.as_deref().unwrap_or_default()),
            Prompt::Commit | Prompt::NewBranch => String::new(),
        };
        self.input.update(cx, |input, cx| {
            input.set_text(start, cx);
            input.set_placeholder(prompt.placeholder(), cx);
        });
        self.remote.update(cx, |input, cx| input.clear(cx));
        self.body.update(cx, |input, cx| input.clear(cx));
        self.publish_private = true;
        // GitHub when its command-line tool is there, else a remote of one's own.
        self.publish_way = if status.gh {
            PublishWay::GitHub
        } else {
            PublishWay::Remote
        };
        self.focus_field(window, cx);
        cx.notify();
    }

    /// The keys to the field the question is waiting on.
    fn focus_field(&self, window: &mut Window, cx: &mut Context<Self>) {
        let field = match (self.menu, self.publish_way) {
            (Menu::Prompt(Prompt::Publish), PublishWay::Remote) => &self.remote,
            (Menu::Prompt(Prompt::Publish), PublishWay::Local) => {
                window.focus(&self.focus_handle);
                return;
            }
            _ => &self.input,
        };
        field.read(cx).focus(window);
    }

    fn choose_way(&mut self, way: PublishWay, window: &mut Window, cx: &mut Context<Self>) {
        self.publish_way = way;
        self.focus_field(window, cx);
        cx.notify();
    }

    /// What the question sends as it stands; None while it cannot be sent.
    fn command(&self, prompt: Prompt, cx: &App) -> Option<Value> {
        let words = self.input.read(cx).text().trim().to_owned();
        match prompt {
            Prompt::Publish => publish_command(
                self.publish_way,
                &words,
                self.publish_private,
                self.remote.read(cx).text(),
            ),
            Prompt::Commit => Some(json!({"type": "collab_commit", "message": words})),
            Prompt::NewBranch => {
                let name = branch_name(&words);
                (!name.is_empty())
                    .then(|| json!({"type": "collab_branch", "name": name, "create": true}))
            }
            Prompt::PullRequest => pull_request_command(&words, self.body.read(cx).text()),
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Menu::Prompt(prompt) = self.menu else {
            return;
        };
        let Some(command) = self.command(prompt, cx) else {
            return;
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
        // The kit's trigger: another menu open goes on its press, and
        // pressed again, its own menu goes rather than opening anew.
        let mut chip = theme::type_style(
            ui::trigger(
                "collab-chip",
                cx.listener(|this, _: &gpui::MouseDownEvent, window, cx| this.toggle(window, cx)),
            ),
            text::CONTROL,
        )
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
        // A Tab stop of its own: ringed while it has the keys, Enter or
        // Space opens its menu; a press leaves the keys where they were.
        .relative()
        .track_focus(&self.chip_focus)
        .key_context(ui::CONTROL)
        .capture_any_mouse_down(|_, window, _| window.prevent_default())
        .on_action(|_: &ui::FocusNext, window, cx| ui::tab_to(true, window, cx))
        .on_action(|_: &ui::FocusPrevious, window, cx| ui::tab_to(false, window, cx))
        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, _| {
            let keystroke = &event.keystroke;
            if matches!(keystroke.key.as_str(), "enter" | "space")
                && !keystroke.modifiers.modified()
            {
                this.chip_armed = true;
            }
        }))
        .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
            if event.is_keyboard() && std::mem::take(&mut this.chip_armed) {
                this.toggle(window, cx);
            }
        }))
        .when(self.chip_focus.is_focused(window), |chip| {
            chip.child(ui::ring(radius::CONTROL))
        });
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
        let mark = if self.failure.last().is_some() && !busy {
            Some(theme::neg())
        } else if status.changed && !busy && !conflicted {
            Some(theme::ink2())
        } else {
            None
        };
        let tooltip: SharedString = match (self.failure.last(), status.changed) {
            (Some(failure), _) => format!("Sync failed: {failure}").into(),
            _ if conflicted => "A merge is unfinished; finish it with git".into(),
            (None, true) => "Edits not committed yet".into(),
            _ => "Sync".into(),
        };
        // Its menu says all the tip would, so the tip waits while it is open.
        ui::tip_on(
            chip.children(mark.map(|color| ui::dot(color, 6.0))),
            (!open).then(|| ui::Tip::new(tooltip)),
        )
    }

    /// A question asked in the menu's place (Commit…, New Branch…, Pull
    /// Request…, Publish to GitHub…), rising into place as it fades in.
    fn question(&self, prompt: Prompt, window: &Window, cx: &Context<Self>) -> AnyElement {
        let width = if prompt.wide() {
            PROMPT_WIDTH
        } else {
            MENU_WIDTH
        };
        let rise = scene::MENU_RISE;
        div()
            .key_context(MENU_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, window, cx| this.back(window, cx)))
            // A press anywhere else puts it away, and does nothing more: a
            // press on the chip closes it rather than opening it again.
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .occlude()
            .relative()
            .w(px(width))
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
            .child(self.prompt(prompt, window, cx))
            .with_animation(
                ("collab-question", self.menus_opened),
                motion::animation(scene::FADE),
                move |menu, t| menu.opacity(t).top(px(rise * (1.0 - t))),
            )
            .into_any_element()
    }

    /// The menu's head, and the line under it.
    fn head_and_line(&self, cx: &App) -> AnyElement {
        let status = self.status.clone().unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .child(self.head(&status, cx))
            .child(ui::hairline(true).mb(px(theme::space::S4)))
            .into_any_element()
    }

    /// The menu's head: where the project stands, and what went wrong.
    fn head(&self, status: &GitStatus, cx: &App) -> impl IntoElement {
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
                .last()
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

    /// Why each of the menu's commands cannot run now, as the registry says
    /// it for the whole window (a project.kerf changed on disk holds back
    /// Sync and the branches); while a sync runs, that.
    fn reasons(&self, cx: &App) -> impl Fn(&str) -> Result<(), SharedString> {
        let states = kerf_bridge::commands_states(&self.session.read(cx).command_context());
        let busy = self.busy();
        move |id: &str| {
            if busy && id != "collab.live" {
                return Err("A sync is running".into());
            }
            match crate::palette::unavailable(&states, id) {
                Some(reason) => Err(reason.to_owned().into()),
                None => Ok(()),
            }
        }
    }

    /// The menu's rows as things stand: Sync Now, Live Sync (ticked while
    /// on), Commit…; Switch Branch ▸ (the other branches, or No Other
    /// Branches), New Branch…, Pull Request…, Publish to GitHub… while
    /// there is no remote; Consolidate Media and Refresh. A project that is
    /// no repository yet has Publish to GitHub… and Make a Repository Here.
    /// A row that cannot run is greyed, and says why in its tip.
    fn items(&self, cx: &App) -> Vec<ui::MenuItem> {
        let status = self.status.clone().unwrap_or_default();
        let reason = self.reasons(cx);
        let this = self.this.clone();
        let row = |words: &'static str, enabled: Result<(), SharedString>| {
            let item = ui::MenuItem::new(words);
            match enabled {
                Ok(()) => item,
                Err(why) => item.disabled(true).tip(why),
            }
        };
        let asks = |prompt: Prompt| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |collab, cx| collab.ask(prompt, true, window, cx));
            }
        };
        let runs = |command: Value| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |collab, cx| collab.run(command.clone(), window, cx));
            }
        };
        let mut items: Vec<ui::MenuItem> = Vec::new();
        if !status.repository {
            items.push(
                row("Publish to GitHub…", reason("collab.publish"))
                    .on_choose(asks(Prompt::Publish)),
            );
            items.push(
                row("Make a Repository Here", reason("collab.publish"))
                    .on_choose(runs(json!({"type": "collab_publish"}))),
            );
        } else {
            let live = status.live;
            let set_live = this.clone();
            items.push(
                row("Sync Now", reason("collab.sync"))
                    .on_choose(runs(json!({"type": "collab_sync"}))),
            );
            items.push(
                row("Live Sync", reason("collab.live"))
                    .checked(live)
                    .on_choose(move |_, cx| {
                        let _ = set_live.update(cx, |collab, cx| collab.set_live(!live, cx));
                    }),
            );
            items.push(row("Commit…", reason("collab.commit")).on_choose(asks(Prompt::Commit)));
            // Switch Branch opens on the other branches, as Qt's did; with
            // none it says so, and holds back only for what holds back a sync.
            let others = other_branches(status.branch.as_deref(), &status.branches);
            let switch = if others.is_empty() {
                reason("collab.sync")
            } else {
                reason("collab.switchBranch")
            };
            let rows = if others.is_empty() {
                vec![ui::MenuItem::new(NO_OTHER_BRANCHES).disabled(true)]
            } else {
                others
                    .into_iter()
                    .map(|name| {
                        ui::MenuItem::new(name.clone()).on_choose(runs(
                            json!({"type": "collab_branch", "name": name, "create": false}),
                        ))
                    })
                    .collect()
            };
            items.push(row("Switch Branch", switch).separated().submenu(rows));
            items.push(
                row("New Branch…", reason("collab.newBranch")).on_choose(asks(Prompt::NewBranch)),
            );
            items.push(
                row("Pull Request…", reason("collab.pullRequest"))
                    .on_choose(asks(Prompt::PullRequest)),
            );
            if status.remote.is_none() {
                items.push(
                    row("Publish to GitHub…", reason("collab.publish"))
                        .on_choose(asks(Prompt::Publish)),
                );
            }
        }
        items.push(
            row("Consolidate Media", reason("collab.consolidate"))
                .separated()
                .on_choose(runs(json!({"type": "collab_consolidate"}))),
        );
        let refresh = this.clone();
        items.push(row("Refresh", Ok(())).on_choose(move |window, cx| {
            let _ = refresh.update(cx, |collab, cx| {
                collab.session.update(cx, |session, _| {
                    session.send(json!({"type": "collab_status"}))
                });
                collab.close(window, cx);
            });
        }));
        items
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

    /// Publish's three ways, and the fields of the one chosen.
    fn publish_fields(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let status = self.status.clone().unwrap_or_default();
        let this = cx.entity().downgrade();
        let ways = PublishWay::ALL.map(|way| (way, way.label()));
        let mut chooser = ui::segmented(
            "collab-publish-way",
            &ways,
            self.publish_way,
            move |way, window, cx| {
                let _ = this.update(cx, |this, cx| this.choose_way(way, window, cx));
            },
        )
        .tips(&[
            if status.gh { "" } else { NEEDS_GH },
            "",
            if status.repository {
                "The project is a repository already"
            } else {
                ""
            },
        ]);
        if !status.gh {
            chooser = chooser.disabled(PublishWay::GitHub);
        }
        if status.repository {
            chooser = chooser.disabled(PublishWay::Local);
        }
        let this = cx.entity().downgrade();
        let fields = match self.publish_way {
            PublishWay::GitHub => div()
                .flex()
                .items_center()
                .gap(px(theme::space::S8))
                .child(
                    crate::launcher::field(&self.input, window, cx)
                        .flex_1()
                        .min_w_0(),
                )
                .child(ui::label("Private", text::CONTROL, theme::ink2()))
                .child(ui::switch(
                    "collab-publish-private",
                    self.publish_private,
                    move |on, _, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.publish_private = on;
                            cx.notify();
                        });
                    },
                ))
                .into_any_element(),
            PublishWay::Remote => {
                crate::launcher::field(&self.remote, window, cx).into_any_element()
            }
            PublishWay::Local => div().into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .child(div().flex().child(chooser))
            .child(ui::label(self.publish_way.says(), text::CONTROL, theme::ink2()))
            .child(fields)
            .when_some(status.enclosing.filter(|enclosing| !enclosing.is_empty()), |column, enclosing| {
                column.child(ui::label(
                    format!("This folder is inside another repository ({enclosing}); the project gets a repository of its own."),
                    text::META,
                    theme::ink3(),
                ))
            })
            .into_any_element()
    }

    fn prompt(&self, prompt: Prompt, window: &Window, cx: &Context<Self>) -> AnyElement {
        let ready = self.command(prompt, cx).is_some();
        let branch = self
            .status
            .as_ref()
            .and_then(|status| status.branch.clone());
        let fields = match prompt {
            Prompt::Publish => self.publish_fields(window, cx),
            Prompt::PullRequest => div()
                .flex()
                .flex_col()
                .gap(px(theme::space::S6))
                .child(crate::launcher::field(&self.input, window, cx))
                .child(ui::label("Description", text::META_STRONG, theme::ink3()))
                .child(area(&self.body, window, cx))
                .into_any_element(),
            Prompt::Commit | Prompt::NewBranch => {
                crate::launcher::field(&self.input, window, cx).into_any_element()
            }
        };
        div()
            .p(px(theme::space::S8))
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .child(ui::label(prompt.title(), text::TITLE, theme::ink()))
            .child(ui::label(
                prompt.says(branch.as_deref()),
                text::META,
                theme::ink3(),
            ))
            .child(fields)
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

impl EventEmitter<CollabEvent> for Collab {}

impl Render for Collab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        if !session.has_project() || self.status.is_none() {
            return div().into_any_element();
        }
        // The menu bar (macOS) offers the window in front's branches.
        if cfg!(target_os = "macos") && window.is_window_active() {
            if let Some(status) = &self.status {
                let others = other_branches(status.branch.as_deref(), &status.branches);
                let blocked = self.reasons(cx)("collab.sync").is_err() || !status.repository;
                crate::commands::set_menu_branches(
                    crate::commands::MenuBranches { others, blocked },
                    cx,
                );
            }
        }
        let shown = match self.menu {
            Menu::Closed => None,
            Menu::Prompt(prompt) => Some(self.question(prompt, window, cx)),
            Menu::Open | Menu::Branches => self.kit.clone().map(|kit| kit.into_any_element()),
        };
        div()
            .relative()
            .flex_none()
            .child(self.chip(window, cx))
            .when_some(shown, |wrapper, shown| {
                wrapper.child(
                    // Under the chip, its right edge on the chip's, over everything.
                    div().absolute().top_full().right_0().child(
                        deferred(
                            anchored()
                                .anchor(Corner::TopRight)
                                .offset(point(px(0.0), px(theme::space::S4)))
                                .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                                .child(shown),
                        )
                        .with_priority(1),
                    ),
                )
            })
            .into_any_element()
    }
}

// ---------------------------------------------------------- Collab's pieces

/// A field of several lines (a pull request's description), as the
/// launcher's one-line field looks.
fn area(input: &Entity<TextInput>, window: &Window, cx: &App) -> gpui::Div {
    let handle = input.read(cx).focus_handle(cx);
    let focused = handle.is_focused(window);
    div()
        .min_h(px(BODY_LEAST))
        .px(px(theme::space::S8))
        .py(px(theme::space::S6))
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(if focused {
            theme::accent()
        } else {
            theme::line2()
        })
        .on_mouse_down(gpui::MouseButton::Left, move |_, window, _| {
            window.focus(&handle);
            window.prevent_default();
        })
        .child(input.clone())
}

/// What Switch Branch's submenu says with no branch but this one, as Qt's.
pub const NO_OTHER_BRANCHES: &str = "No Other Branches";

/// The branches to switch to from `current`: the others, each once, in
/// the order git lists them.
pub fn other_branches(current: Option<&str>, branches: &[String]) -> Vec<String> {
    let mut others: Vec<String> = Vec::new();
    for branch in branches {
        if !branch.is_empty() && Some(branch.as_str()) != current && !others.contains(branch) {
            others.push(branch.clone());
        }
    }
    others
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
