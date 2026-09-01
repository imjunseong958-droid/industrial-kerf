use cut_runtime::{PendingProposal, StateSnapshot};
use egui::{Color32, RichText, TextEdit};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProposalStatus {
    #[default]
    Empty,
    Pending {
        proposal_id: String,
        base_revision: u64,
        change_count: usize,
        diff_truncated: bool,
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
    Accept(String),
    Reject(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentRailViewModel {
    pub proposal: ProposalStatus,
}

impl AgentRailViewModel {
    pub fn from_snapshot(snapshot: &StateSnapshot) -> Self {
        let proposal = match snapshot.pending_proposal.as_ref() {
            Some(PendingProposal::Pending {
                proposal_id,
                base_revision,
                diff,
                diff_truncated,
                ..
            }) => ProposalStatus::Pending {
                proposal_id: proposal_id.clone(),
                base_revision: *base_revision,
                change_count: diff.len(),
                diff_truncated: *diff_truncated,
            },
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
        Self { proposal }
    }

    pub fn show(&self, ui: &mut egui::Ui, prompt_draft: &mut String) -> AgentRailAction {
        ui.heading(RichText::new("Agent rail").color(Color32::WHITE));
        ui.add_space(6.0);
        ui.add(
            TextEdit::multiline(prompt_draft)
                .desired_rows(3)
                .hint_text("Describe an edit…"),
        );
        if ui.button("Send").clicked() {
            let prompt = prompt_draft.trim().to_owned();
            if !prompt.is_empty() {
                prompt_draft.clear();
                return AgentRailAction::Send(prompt);
            }
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
            } => {
                ui.label(format!("Pending at revision {base_revision}"));
                ui.label(format!("{change_count} proposed change(s)"));
                if *diff_truncated {
                    ui.label("Diff truncated");
                }
                let mut action = AgentRailAction::None;
                ui.horizontal(|ui| {
                    if ui.button("Accept").clicked() {
                        action = AgentRailAction::Accept(proposal_id.clone());
                    }
                    if ui.button("Reject").clicked() {
                        action = AgentRailAction::Reject(proposal_id.clone());
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
