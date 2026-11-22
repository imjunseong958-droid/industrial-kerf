//! Export's Preflight, the last QC pass's checks as Export's menu lists
//! them, and Ask Kerf, which fills the composer with what a failed check
//! asks for.

use super::finishing::finishing;
use editor_session::session::Session;
use gpui::{App, Entity, SharedString, Window};
use serde_json::Value;

/// Fills the conversation's composer with `words`, to be changed before it
/// is sent: a failed check's Ask Kerf, in Export's
/// Preflight and in Finish's QC. The window's root is to hand it to the
/// conversation; until something on the way to the keys takes it
/// ([`can_ask`]), neither offers Ask Kerf.
#[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct AskKerf {
    pub words: SharedString,
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
    for (index, row) in rows.iter().enumerate() {
        if chosen.len() >= least {
            break;
        }
        if passed(row) && !chosen.contains(&index) {
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
