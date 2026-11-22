//! What the window says of what git did: a finished sync's lines in
//! Activity and its notice, what a pull request came to, what a sync is
//! doing while it runs, and the last sync's failure as the window keeps it.

use crate::finish::{plural, sentence};
use kerf_bridge::event::CollabSync;
use serde_json::Value;

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

/// A sync's failure as the window keeps it: why the last sync failed,
/// until one succeeds (the chip's dot and tip, its menu,
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

/// What a sync is doing, as a person says it.
pub(super) fn stage_words(stage: Option<&CollabSync>) -> &'static str {
    match stage {
        Some(CollabSync::Committing) => "Committing…",
        Some(CollabSync::Fetching) => "Pulling…",
        Some(CollabSync::Merging) => "Merging…",
        Some(CollabSync::Pushing) => "Pushing…",
        Some(CollabSync::Downloading) => "Downloading media…",
        _ => "Syncing…",
    }
}

/// What a finished sync did, in a line.
pub(super) fn summary(report: &Value) -> String {
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
