use cut_agent::{AgentEvent, AgentTargetSummary, ToolResultSummary};
use cut_runtime::{PendingProposal, StateSnapshot};
use egui::{Color32, RichText, TextEdit};
use std::collections::VecDeque;

/// Maximum number of retained entries for the latest agent run.
pub const MAX_AGENT_RUN_LOG_ENTRIES: usize = 128;
/// Maximum accumulated agent text retained by the desktop view model.
pub const MAX_AGENT_RUN_TEXT_BYTES: usize = 16 * 1024;
const MAX_AGENT_LOG_TEXT_BYTES: usize = 4 * 1024;
const MAX_AGENT_TARGET_ITEMS: usize = 8;
const MAX_PROPOSAL_IMPACT_ROWS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalImpactKind {
    Track,
    Clip,
    Overlay,
    Project,
    Unclassified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalImpactRow {
    pub kind: ProposalImpactKind,
    pub entity_id: Option<String>,
    pub summary: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProposalStatus {
    #[default]
    Empty,
    Pending {
        proposal_id: String,
        base_revision: u64,
        change_count: usize,
        diff_truncated: bool,
        impacts: Option<Vec<ProposalImpactRow>>,
        impacts_truncated: bool,
    },
    Accepted {
        proposal_id: String,
        applied_revision: u64,
    },
    Rejected {
        proposal_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRailAction {
    None,
    Send(String),
    Abort,
    Accept(String),
    Reject(String),
    RequestChanges {
        proposal_id: String,
        feedback: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
/// Lifecycle state for the latest desktop agent run.
pub enum AgentRunState {
    #[default]
    Idle,
    Running,
    AwaitingReview {
        proposal_id: String,
    },
    Completed {
        proposal_id: Option<String>,
    },
    Failed {
        message: String,
    },
    Aborted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// One bounded, display-ready entry in the desktop agent log.
pub enum AgentRunLogEntry {
    Prompt(String),
    Text(String),
    TurnStarted {
        stop_reason: String,
    },
    ToolStarted {
        id: String,
        name: String,
        targets: AgentTargetSummary,
    },
    ToolFinished {
        id: String,
        name: String,
        ok: bool,
        summary: ToolResultSummary,
    },
    Error(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
/// Transient agent run state owned by the desktop shell.
pub struct AgentRunViewModel {
    pub state: AgentRunState,
    pub log: VecDeque<AgentRunLogEntry>,
    pub final_text: String,
    pub proposal_id: Option<String>,
    proposal_at_start: Option<String>,
    proposal_match_pending: bool,
}

impl AgentRunViewModel {
    /// Start a new run and retain the proposal ID that existed before it.
    pub fn begin(&mut self, prompt: &str, existing_proposal_id: Option<String>) {
        *self = Self {
            state: AgentRunState::Running,
            proposal_at_start: existing_proposal_id,
            ..Self::default()
        };
        self.push(AgentRunLogEntry::Prompt(bounded_text(
            prompt,
            MAX_AGENT_LOG_TEXT_BYTES,
        )));
    }

    /// Apply one streamed session event to the bounded desktop projection.
    pub fn apply_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta(text) => self.append_text(&text),
            AgentEvent::TurnStarted { stop_reason } => {
                self.push(AgentRunLogEntry::TurnStarted {
                    stop_reason: bounded_text(&stop_reason, MAX_AGENT_LOG_TEXT_BYTES),
                });
            }
            AgentEvent::ToolCallStarted { id, name, targets } => {
                self.push(AgentRunLogEntry::ToolStarted {
                    id: bounded_text(&id, MAX_AGENT_LOG_TEXT_BYTES),
                    name: bounded_text(&name, MAX_AGENT_LOG_TEXT_BYTES),
                    targets: compact_targets(targets),
                });
            }
            AgentEvent::ToolCallFinished {
                id,
                name,
                ok,
                summary,
            } => {
                self.push(AgentRunLogEntry::ToolFinished {
                    id: bounded_text(&id, MAX_AGENT_LOG_TEXT_BYTES),
                    name: bounded_text(&name, MAX_AGENT_LOG_TEXT_BYTES),
                    ok,
                    summary: compact_summary(summary),
                });
            }
            AgentEvent::Done { text } => {
                self.final_text = bounded_text(&text, MAX_AGENT_RUN_TEXT_BYTES);
                if self
                    .log
                    .iter()
                    .all(|entry| !matches!(entry, AgentRunLogEntry::Text(_)))
                    && !self.final_text.is_empty()
                {
                    self.push(AgentRunLogEntry::Text(self.final_text.clone()));
                }
                if self.is_running() {
                    self.state = AgentRunState::Completed {
                        proposal_id: self.proposal_id.clone(),
                    };
                    self.proposal_match_pending = true;
                }
            }
            AgentEvent::Aborted => self.state = AgentRunState::Aborted,
        }
    }

    /// Record a terminal agent or provider failure without retaining raw data.
    pub fn fail(&mut self, message: &str) {
        let message = bounded_text(message, MAX_AGENT_LOG_TEXT_BYTES);
        self.push(AgentRunLogEntry::Error(message.clone()));
        self.state = AgentRunState::Failed { message };
        self.proposal_match_pending = false;
    }

    /// Match a newly refreshed daemon snapshot to the proposal created by
    /// this run. An older proposal present when the run began is ignored.
    pub fn sync_pending_proposal(&mut self, pending: Option<&PendingProposal>) {
        match pending {
            Some(PendingProposal::Pending { proposal_id, .. }) => {
                if self.proposal_id.as_deref() == Some(proposal_id) {
                    if self.is_completed() {
                        self.state = AgentRunState::AwaitingReview {
                            proposal_id: proposal_id.clone(),
                        };
                    }
                    return;
                }
                if matches!(self.state, AgentRunState::Completed { proposal_id: None }) {
                    if self.proposal_match_pending
                        && self.proposal_at_start.as_deref() != Some(proposal_id)
                    {
                        self.proposal_id = Some(proposal_id.clone());
                        self.state = AgentRunState::AwaitingReview {
                            proposal_id: proposal_id.clone(),
                        };
                    }
                    self.proposal_match_pending = false;
                }
            }
            Some(PendingProposal::Accepted { proposal_id, .. })
            | Some(PendingProposal::Rejected { proposal_id, .. })
                if self.proposal_id.as_deref() == Some(proposal_id) =>
            {
                self.state = AgentRunState::Completed {
                    proposal_id: Some(proposal_id.clone()),
                };
            }
            None if matches!(self.state, AgentRunState::Completed { proposal_id: None }) => {
                self.proposal_match_pending = false;
            }
            _ => {}
        }
    }

    /// Return whether another prompt must be rejected while this run works.
    pub fn is_running(&self) -> bool {
        matches!(self.state, AgentRunState::Running)
    }

    /// Return whether the latest run ended through the abort path.
    pub fn is_aborted(&self) -> bool {
        matches!(self.state, AgentRunState::Aborted)
    }

    /// Return whether the run reached a successful terminal state.
    pub fn is_completed(&self) -> bool {
        matches!(
            self.state,
            AgentRunState::Completed { .. } | AgentRunState::AwaitingReview { .. }
        )
    }

    /// Return the proposal awaiting review, if this run staged one.
    pub fn awaiting_proposal_id(&self) -> Option<&str> {
        match &self.state {
            AgentRunState::AwaitingReview { proposal_id } => Some(proposal_id),
            AgentRunState::Idle
            | AgentRunState::Running
            | AgentRunState::Completed { .. }
            | AgentRunState::Failed { .. }
            | AgentRunState::Aborted => None,
        }
    }

    /// Format a compact status line for the rail.
    pub fn status_text(&self) -> String {
        match &self.state {
            AgentRunState::Idle => "Idle".into(),
            AgentRunState::Running => "Running".into(),
            AgentRunState::AwaitingReview { proposal_id } => {
                format!("Awaiting review · {proposal_id}")
            }
            AgentRunState::Completed { proposal_id } => proposal_id
                .as_deref()
                .map_or_else(|| "Completed".into(), |id| format!("Completed · {id}")),
            AgentRunState::Failed { message } => format!("Failed · {message}"),
            AgentRunState::Aborted => "Aborted".into(),
        }
    }

    fn append_text(&mut self, text: &str) {
        let Some(last) = self.log.back_mut() else {
            self.push(AgentRunLogEntry::Text(bounded_text(
                text,
                MAX_AGENT_RUN_TEXT_BYTES,
            )));
            return;
        };
        if let AgentRunLogEntry::Text(current) = last {
            append_bounded(current, text, MAX_AGENT_RUN_TEXT_BYTES);
        } else {
            self.push(AgentRunLogEntry::Text(bounded_text(
                text,
                MAX_AGENT_RUN_TEXT_BYTES,
            )));
        }
    }

    fn push(&mut self, entry: AgentRunLogEntry) {
        if self.log.len() >= MAX_AGENT_RUN_LOG_ENTRIES {
            self.log.pop_front();
        }
        self.log.push_back(entry);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
/// Combined proposal and latest-run projection rendered by the agent rail.
pub struct AgentRailViewModel {
    pub proposal: ProposalStatus,
    pub run: AgentRunViewModel,
}

impl AgentRailViewModel {
    /// Project only the proposal state from an authoritative snapshot.
    pub fn from_snapshot(snapshot: &StateSnapshot) -> Self {
        Self::from_snapshot_with_run(snapshot, AgentRunViewModel::default())
    }

    /// Project a snapshot while preserving transient run history.
    pub fn from_snapshot_with_run(snapshot: &StateSnapshot, mut run: AgentRunViewModel) -> Self {
        let proposal = match snapshot.pending_proposal.as_ref() {
            Some(PendingProposal::Pending {
                proposal_id,
                base_revision,
                diff,
                diff_truncated,
                impacts,
                ..
            }) => {
                let impacts_truncated = impacts
                    .as_ref()
                    .is_some_and(|items| items.len() > MAX_PROPOSAL_IMPACT_ROWS);
                ProposalStatus::Pending {
                    proposal_id: proposal_id.clone(),
                    base_revision: *base_revision,
                    change_count: diff.len(),
                    diff_truncated: *diff_truncated,
                    impacts: impacts.as_ref().map(|items| {
                        items
                            .iter()
                            .take(MAX_PROPOSAL_IMPACT_ROWS)
                            .map(proposal_impact_row)
                            .collect()
                    }),
                    impacts_truncated,
                }
            }
            Some(PendingProposal::Accepted {
                proposal_id,
                applied_revision,
                ..
            }) => ProposalStatus::Accepted {
                proposal_id: proposal_id.clone(),
                applied_revision: *applied_revision,
            },
            Some(PendingProposal::Rejected { proposal_id, .. }) => ProposalStatus::Rejected {
                proposal_id: proposal_id.clone(),
            },
            None => ProposalStatus::Empty,
        };
        run.sync_pending_proposal(snapshot.pending_proposal.as_ref());
        Self { proposal, run }
    }

    /// Begin a run while remembering the currently pending proposal.
    pub fn begin_run(&mut self, prompt: &str) {
        self.run.begin(prompt, self.pending_proposal_id());
    }

    /// Apply one streamed agent event to the run projection.
    pub fn apply_run_event(&mut self, event: AgentEvent) {
        self.run.apply_event(event);
    }

    /// Record a terminal run failure.
    pub fn fail_run(&mut self, message: &str) {
        self.run.fail(message);
    }

    /// Reconcile a refreshed proposal lifecycle with the current run.
    pub fn sync_pending_proposal(&mut self, pending: Option<&PendingProposal>) {
        self.run.sync_pending_proposal(pending);
    }

    /// Return the currently pending proposal ID, if any.
    pub fn pending_proposal_id(&self) -> Option<String> {
        match &self.proposal {
            ProposalStatus::Pending { proposal_id, .. } => Some(proposal_id.clone()),
            ProposalStatus::Empty
            | ProposalStatus::Accepted { .. }
            | ProposalStatus::Rejected { .. } => None,
        }
    }

    pub fn show(&self, ui: &mut egui::Ui, prompt_draft: &mut String) -> AgentRailAction {
        ui.heading(RichText::new("Agent rail").color(Color32::WHITE));
        ui.add_space(6.0);
        ui.add(
            TextEdit::multiline(prompt_draft)
                .desired_rows(3)
                .hint_text("Describe an edit…"),
        );
        if ui
            .add_enabled(!self.run.is_running(), egui::Button::new("Send"))
            .clicked()
        {
            let prompt = prompt_draft.trim().to_owned();
            if !prompt.is_empty() {
                prompt_draft.clear();
                return AgentRailAction::Send(prompt);
            }
        }
        if self.run.is_running() && ui.button("Abort").clicked() {
            return AgentRailAction::Abort;
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Run").strong());
        ui.label(self.run.status_text());
        if !self.run.log.is_empty() {
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for entry in &self.run.log {
                        show_log_entry(ui, entry);
                    }
                });
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Proposal").strong());

        match &self.proposal {
            ProposalStatus::Empty => {
                ui.label(RichText::new("No pending proposal").color(Color32::from_gray(145)));
                AgentRailAction::None
            }
            ProposalStatus::Pending {
                proposal_id,
                base_revision,
                change_count,
                diff_truncated,
                impacts,
                impacts_truncated,
            } => {
                ui.label(format!("Pending at revision {base_revision}"));
                match impacts {
                    Some(impacts) if impacts.is_empty() => {
                        ui.label("No semantic impacts");
                    }
                    Some(impacts) => {
                        for impact in impacts {
                            ui.label(format!("• {}", impact.summary));
                        }
                        if *impacts_truncated {
                            ui.label("Additional semantic impacts omitted");
                        }
                    }
                    None => {
                        ui.label(format!(
                            "Semantic impacts unavailable · {change_count} raw change(s)"
                        ));
                    }
                }
                if *diff_truncated {
                    ui.label("Raw diagnostic diff truncated");
                }
                let mut action = AgentRailAction::None;
                ui.horizontal(|ui| {
                    if ui.button("Accept").clicked() {
                        action = AgentRailAction::Accept(proposal_id.clone());
                    }
                    if ui.button("Reject").clicked() {
                        action = AgentRailAction::Reject(proposal_id.clone());
                    }
                    if ui
                        .add_enabled(
                            !prompt_draft.trim().is_empty(),
                            egui::Button::new("Request changes"),
                        )
                        .on_hover_text("Reject this proposal and ask the agent to revise it")
                        .clicked()
                    {
                        let feedback = prompt_draft.trim().to_owned();
                        prompt_draft.clear();
                        action = AgentRailAction::RequestChanges {
                            proposal_id: proposal_id.clone(),
                            feedback,
                        };
                    }
                });
                action
            }
            ProposalStatus::Accepted {
                proposal_id,
                applied_revision,
            } => {
                ui.label(format!(
                    "Accepted {proposal_id} at revision {applied_revision}"
                ));
                AgentRailAction::None
            }
            ProposalStatus::Rejected { proposal_id } => {
                ui.label(format!("Rejected {proposal_id}"));
                AgentRailAction::None
            }
        }
    }
}

fn proposal_impact_row(impact: &cut_runtime::ProposalImpact) -> ProposalImpactRow {
    use cut_runtime::ProposalImpact;
    let (kind, entity_id, summary) = match impact {
        ProposalImpact::TrackAdded { track_id, index } => (
            ProposalImpactKind::Track,
            Some(track_id.clone()),
            format!("Track {track_id} added at row {}", index + 1),
        ),
        ProposalImpact::TrackRemoved { track_id, .. } => (
            ProposalImpactKind::Track,
            Some(track_id.clone()),
            format!("Track {track_id} removed"),
        ),
        ProposalImpact::TrackReordered {
            track_id,
            from_index,
            to_index,
        } => (
            ProposalImpactKind::Track,
            Some(track_id.clone()),
            format!(
                "Track {track_id} moved from row {} to {}",
                from_index + 1,
                to_index + 1
            ),
        ),
        ProposalImpact::ClipAdded {
            clip_id,
            track_id,
            timeline,
            ..
        } => (
            ProposalImpactKind::Clip,
            Some(clip_id.clone()),
            format!(
                "Clip {clip_id} added to {track_id} at {:.3}–{:.3}s",
                timeline.start, timeline.end
            ),
        ),
        ProposalImpact::ClipRemoved {
            clip_id, track_id, ..
        } => (
            ProposalImpactKind::Clip,
            Some(clip_id.clone()),
            format!("Clip {clip_id} removed from {track_id}"),
        ),
        ProposalImpact::ClipMoved {
            clip_id,
            from_track_id,
            to_track_id,
            from,
            to,
        } => (
            ProposalImpactKind::Clip,
            Some(clip_id.clone()),
            format!(
                "Clip {clip_id} moved {from_track_id} {:.3}s → {to_track_id} {:.3}s",
                from.start, to.start
            ),
        ),
        ProposalImpact::ClipTrimmed {
            clip_id,
            before,
            after,
            ..
        } => (
            ProposalImpactKind::Clip,
            Some(clip_id.clone()),
            format!(
                "Clip {clip_id} trimmed {:.3}–{:.3}s → {:.3}–{:.3}s",
                before.start, before.end, after.start, after.end
            ),
        ),
        ProposalImpact::ClipPropertyChanged {
            clip_id, property, ..
        } => (
            ProposalImpactKind::Clip,
            Some(clip_id.clone()),
            format!("Clip {clip_id} changed {property}"),
        ),
        ProposalImpact::OverlayAdded {
            overlay_id,
            overlay_kind,
            ..
        } => (
            ProposalImpactKind::Overlay,
            Some(overlay_id.clone()),
            format!("{overlay_kind} overlay {overlay_id} added"),
        ),
        ProposalImpact::OverlayRemoved {
            overlay_id,
            overlay_kind,
            ..
        } => (
            ProposalImpactKind::Overlay,
            Some(overlay_id.clone()),
            format!("{overlay_kind} overlay {overlay_id} removed"),
        ),
        ProposalImpact::OverlayChanged {
            overlay_id,
            overlay_kind,
            ..
        } => (
            ProposalImpactKind::Overlay,
            Some(overlay_id.clone()),
            format!("{overlay_kind} overlay {overlay_id} changed"),
        ),
        ProposalImpact::ProjectSettingChanged { property, .. } => (
            ProposalImpactKind::Project,
            None,
            format!("Project setting {property} changed"),
        ),
        ProposalImpact::Unclassified { path, .. } => (
            ProposalImpactKind::Unclassified,
            None,
            format!("Unclassified change at {path}"),
        ),
    };
    ProposalImpactRow {
        kind,
        entity_id,
        summary,
    }
}

fn show_log_entry(ui: &mut egui::Ui, entry: &AgentRunLogEntry) {
    match entry {
        AgentRunLogEntry::Prompt(text) => {
            ui.label(RichText::new(format!("You · {text}")).color(Color32::from_gray(175)));
        }
        AgentRunLogEntry::Text(text) => {
            ui.label(RichText::new(format!("Agent · {text}")).color(Color32::WHITE));
        }
        AgentRunLogEntry::TurnStarted { stop_reason } => {
            ui.label(
                RichText::new(format!("Turn · {stop_reason}"))
                    .small()
                    .color(Color32::from_gray(135)),
            );
        }
        AgentRunLogEntry::ToolStarted { name, targets, .. } => {
            let targets = targets_text(targets);
            let suffix = if targets.is_empty() {
                String::new()
            } else {
                format!(" · {targets}")
            };
            ui.label(
                RichText::new(format!("▸ {name}{suffix}"))
                    .small()
                    .color(Color32::from_rgb(120, 190, 255)),
            );
        }
        AgentRunLogEntry::ToolFinished {
            name, ok, summary, ..
        } => {
            let marker = if *ok { "✓" } else { "✗" };
            let details = summary_text(summary);
            ui.label(
                RichText::new(format!("{marker} {name} · {details}"))
                    .small()
                    .color(if *ok {
                        Color32::from_gray(175)
                    } else {
                        Color32::from_rgb(255, 125, 125)
                    }),
            );
        }
        AgentRunLogEntry::Error(message) => {
            ui.label(
                RichText::new(format!("Error · {message}")).color(Color32::from_rgb(255, 125, 125)),
            );
        }
    }
}

fn targets_text(targets: &AgentTargetSummary) -> String {
    let mut parts = Vec::new();
    add_target_part(&mut parts, "source", &targets.sources);
    add_target_part(&mut parts, "clip", &targets.clips);
    add_target_part(&mut parts, "track", &targets.tracks);
    add_target_part(&mut parts, "overlay", &targets.overlays);
    add_target_part(&mut parts, "job", &targets.jobs);
    parts.join(" · ")
}

fn add_target_part(parts: &mut Vec<String>, label: &str, values: &[String]) {
    if !values.is_empty() {
        parts.push(format!("{label}: {}", values.join(", ")));
    }
}

fn summary_text(summary: &ToolResultSummary) -> String {
    let mut parts = Vec::new();
    if let Some(status) = &summary.status {
        parts.push(status.clone());
    }
    if let Some(revision) = summary.revision {
        parts.push(format!("revision {revision}"));
    }
    if let Some(count) = summary.count {
        parts.push(format!("{count} item(s)"));
    }
    let targets = targets_text(&summary.targets);
    if !targets.is_empty() {
        parts.push(targets);
    }
    if !summary.artifact_paths.is_empty() {
        parts.push(format!("artifact: {}", summary.artifact_paths.join(", ")));
    }
    if let Some(error) = &summary.error {
        parts.push(error.clone());
    }
    if summary.truncated {
        parts.push("summary truncated".into());
    }
    if parts.is_empty() {
        "completed".into()
    } else {
        parts.join(" · ")
    }
}

fn compact_targets(mut targets: AgentTargetSummary) -> AgentTargetSummary {
    compact_list(&mut targets.sources);
    compact_list(&mut targets.clips);
    compact_list(&mut targets.tracks);
    compact_list(&mut targets.overlays);
    compact_list(&mut targets.jobs);
    targets
}

fn compact_summary(mut summary: ToolResultSummary) -> ToolResultSummary {
    summary.targets = compact_targets(summary.targets);
    compact_list(&mut summary.artifact_paths);
    if let Some(status) = summary.status.as_mut() {
        *status = bounded_text(status, MAX_AGENT_LOG_TEXT_BYTES);
    }
    if let Some(error) = summary.error.as_mut() {
        *error = bounded_text(error, MAX_AGENT_LOG_TEXT_BYTES);
    }
    summary
}

fn compact_list(values: &mut Vec<String>) {
    values.truncate(MAX_AGENT_TARGET_ITEMS);
    for value in values {
        *value = bounded_text(value, MAX_AGENT_LOG_TEXT_BYTES);
    }
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let suffix = "…";
    let limit = max_bytes.saturating_sub(suffix.len());
    let mut end = 0;
    for (index, character) in value.char_indices() {
        let next = index + character.len_utf8();
        if next > limit {
            break;
        }
        end = next;
    }
    format!("{}{}", &value[..end], suffix)
}

fn append_bounded(current: &mut String, text: &str, max_bytes: usize) {
    let remaining = max_bytes.saturating_sub(current.len());
    if remaining == 0 {
        return;
    }
    if text.len() <= remaining {
        current.push_str(text);
        return;
    }
    current.push_str(&bounded_text(text, remaining));
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_agent::{AgentTargetSummary, ToolResultSummary};
    use serde_json::json;

    fn pending(proposal_id: &str) -> PendingProposal {
        PendingProposal::Pending {
            proposal_id: proposal_id.into(),
            base_revision: 0,
            command: json!({"type": "batch", "commands": []}),
            diff: Vec::new(),
            diff_truncated: false,
            freshness: cut_runtime::ProposalFreshness::Current,
            impacts: None,
        }
    }

    #[test]
    fn streamed_text_is_coalesced_and_log_history_is_bounded() {
        let mut run = AgentRunViewModel::default();
        run.begin("make it tighter", None);
        run.apply_event(AgentEvent::TextDelta("hello".into()));
        run.apply_event(AgentEvent::TextDelta(" world".into()));

        assert!(run
            .log
            .iter()
            .any(|entry| matches!(entry, AgentRunLogEntry::Text(text) if text == "hello world")));

        for index in 0..(MAX_AGENT_RUN_LOG_ENTRIES + 10) {
            run.apply_event(AgentEvent::TurnStarted {
                stop_reason: format!("turn-{index}"),
            });
        }
        assert_eq!(run.log.len(), MAX_AGENT_RUN_LOG_ENTRIES);
    }

    #[test]
    fn completed_run_associates_only_a_new_pending_proposal() {
        let current = pending("new-proposal");
        let mut run = AgentRunViewModel::default();
        run.begin("stage an edit", None);
        run.apply_event(AgentEvent::Done {
            text: "done".into(),
        });
        run.sync_pending_proposal(Some(&current));
        assert_eq!(run.awaiting_proposal_id(), Some("new-proposal"));

        let accepted = PendingProposal::Accepted {
            proposal_id: "new-proposal".into(),
            base_revision: 0,
            applied_revision: 1,
        };
        run.sync_pending_proposal(Some(&accepted));
        assert_eq!(
            run.state,
            AgentRunState::Completed {
                proposal_id: Some("new-proposal".into())
            }
        );

        let mut run_with_old_proposal = AgentRunViewModel::default();
        run_with_old_proposal.begin("stage another edit", Some("old-proposal".into()));
        run_with_old_proposal.apply_event(AgentEvent::Done {
            text: "done".into(),
        });
        let old = pending("old-proposal");
        run_with_old_proposal.sync_pending_proposal(Some(&old));
        run_with_old_proposal.sync_pending_proposal(Some(&current));
        assert!(matches!(
            run_with_old_proposal.state,
            AgentRunState::Completed { proposal_id: None }
        ));
    }

    #[test]
    fn run_event_projection_keeps_tool_data_typed() {
        let mut run = AgentRunViewModel::default();
        run.begin("inspect", None);
        run.apply_event(AgentEvent::ToolCallStarted {
            id: "call-1".into(),
            name: "timeline.query".into(),
            targets: AgentTargetSummary {
                clips: vec!["clip-1".into()],
                ..AgentTargetSummary::default()
            },
        });
        run.apply_event(AgentEvent::ToolCallFinished {
            id: "call-1".into(),
            name: "timeline.query".into(),
            ok: true,
            summary: ToolResultSummary {
                revision: Some(4),
                ..ToolResultSummary::default()
            },
        });

        assert!(run.log.iter().any(|entry| matches!(
            entry,
            AgentRunLogEntry::ToolStarted { targets, .. }
                if targets.clips == vec!["clip-1"]
        )));
        assert!(run.log.iter().any(|entry| matches!(
            entry,
            AgentRunLogEntry::ToolFinished { summary, .. }
                if summary.revision == Some(4)
        )));
    }

    #[test]
    fn abort_and_failure_have_distinct_terminal_states() {
        let mut aborted = AgentRunViewModel::default();
        aborted.begin("stop", None);
        aborted.apply_event(AgentEvent::Aborted);
        assert_eq!(aborted.state, AgentRunState::Aborted);

        let mut failed = AgentRunViewModel::default();
        failed.begin("retry", None);
        failed.fail("provider unavailable");
        assert_eq!(
            failed.state,
            AgentRunState::Failed {
                message: "provider unavailable".into()
            }
        );
    }

    #[test]
    fn proposal_projection_keeps_bounded_typed_semantic_impacts() {
        let impacts = (0..(MAX_PROPOSAL_IMPACT_ROWS + 1))
            .map(|index| cut_runtime::ProposalImpact::ClipAdded {
                clip_id: format!("clip-{index}"),
                track_id: "video-1".into(),
                timeline: cut_runtime::ProposalTimeRange {
                    start: index as f64,
                    end: index as f64 + 1.0,
                },
                source: None,
            })
            .collect();
        let snapshot = StateSnapshot {
            project: json!({"version": 2, "tracks": []}),
            revision: 0,
            duration: 0.0,
            can_undo: false,
            can_redo: false,
            sources: Vec::new(),
            pending_proposal: Some(PendingProposal::Pending {
                proposal_id: "proposal-1".into(),
                base_revision: 0,
                command: json!({"type": "batch", "commands": []}),
                diff: Vec::new(),
                diff_truncated: false,
                freshness: cut_runtime::ProposalFreshness::Current,
                impacts: Some(impacts),
            }),
            source_version: 2,
            projection_status: "current".into(),
        };

        let model = AgentRailViewModel::from_snapshot(&snapshot);
        let ProposalStatus::Pending {
            impacts: Some(impacts),
            impacts_truncated,
            ..
        } = model.proposal
        else {
            panic!("expected typed pending proposal")
        };
        assert_eq!(impacts.len(), MAX_PROPOSAL_IMPACT_ROWS);
        assert_eq!(impacts[0].kind, ProposalImpactKind::Clip);
        assert_eq!(impacts[0].entity_id.as_deref(), Some("clip-0"));
        assert!(impacts_truncated);
    }
}
