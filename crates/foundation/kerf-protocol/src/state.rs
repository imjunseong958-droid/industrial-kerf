//! What a client sees of a project: the state kerfd answers with, and the proposals in it.

use crate::{Author, ProjectionStatus, ProposalStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `StateSnapshot::source_version` of the canonical `.kerf` document; 1 and 2
/// were earlier project formats.
pub const SOURCE_VERSION: u8 = 3;

fn default_source_version() -> u8 {
    SOURCE_VERSION
}

fn default_projection_status() -> ProjectionStatus {
    ProjectionStatus::Current
}

/// The most proposals kept at once, open and settled together.
pub const MAX_PROPOSALS: usize = 8;

/// What clients see of the runtime. `document` is the canonical `.kerf`
/// text and `timeline` a [`TimelineView`](crate::TimelineView) of it for editors.
///
/// The fields after `import_warnings` are left out while empty, so the
/// receipts older journals keep still read back byte for byte. For the same
/// reason `timeline` stays the JSON it was written as: a receipt's snapshot
/// is checksummed as stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    pub document: String,
    pub timeline: Value,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub duration: f64,
    pub pending_proposal: Option<PendingProposal>,
    /// 3 for the canonical `.kerf` document.
    #[serde(default = "default_source_version")]
    pub source_version: u8,
    #[serde(default = "default_projection_status")]
    pub projection_status: ProjectionStatus,
    /// What opening the project changed or left behind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import_warnings: Vec<String>,
    /// Every proposal the runtime keeps, newest first: the open ones and
    /// the last few settled. `pending_proposal` is the first open one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposals: Vec<ProposalEntry>,
    /// What a kerfd with review notes wrote here. Nothing sets it now; it is
    /// kept so that the journal records that carry it, whose checksums cover
    /// it, still verify.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub review_revision: u64,
    /// When the current revision was made, RFC 3339; absent for a project
    /// last edited by a kerfd that did not record it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_time: Option<String>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// One version of a proposal: v1, and each `revise` after it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalVersion {
    pub version: u32,
    pub proposal_id: String,
    pub base_revision: u64,
    pub script: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    pub document_diff: String,
}

/// A proposal as clients list it: its lifecycle, who made it, and its
/// versions. A settled one keeps the script and diff of the version that
/// was accepted or rejected.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalEntry {
    /// The current version's id; for an accepted proposal, the version
    /// accepted.
    pub proposal_id: String,
    pub status: ProposalStatus,
    pub freshness: ProposalFreshness,
    pub base_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_revision: Option<u64>,
    pub script: String,
    #[serde(default)]
    pub source: String,
    pub document_diff: String,
    pub author: Author,
    pub version: u32,
    /// The proposal id this version replaced.
    #[serde(default)]
    pub supersedes: Option<String>,
    /// Every version, oldest first, the current one included.
    #[serde(default)]
    pub versions: Vec<ProposalVersion>,
    /// When the current version was staged, RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged: Option<String>,
}

impl ProposalEntry {
    /// Still open: neither accepted nor rejected.
    pub fn is_pending(&self) -> bool {
        self.status == ProposalStatus::Pending
    }

    /// Made on a revision the project has moved on from.
    pub fn is_stale(&self) -> bool {
        self.freshness == ProposalFreshness::Stale
    }

    /// The lines its document diff adds and removes, without the diff's
    /// headers.
    pub fn diff_counts(&self) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;
        for line in self.document_diff.lines() {
            if line.starts_with("+++") || line.starts_with("---") {
                continue;
            }
            if line.starts_with('+') {
                added += 1;
            } else if line.starts_with('-') {
                removed += 1;
            }
        }
        (added, removed)
    }
}

/// Whether a pending proposal still targets the canonical project revision.
///
/// Older checkpoints omit this field and therefore deserialize as `Current`.
/// Runtime snapshots always recompute it from the proposal base revision so
/// every adapter receives the same explicit stale state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalFreshness {
    #[default]
    Current,
    Stale,
}

impl ProposalFreshness {
    fn is_current(value: &Self) -> bool {
        *value == Self::Current
    }
}

/// Proposal lifecycle owned by the canonical runtime.  Terminal states stay
/// in memory so repeating an accept/reject request converges without another
/// project mutation; only `Pending` exposes the proposed script and diff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum PendingProposal {
    Pending {
        #[serde(rename = "proposalId", alias = "proposal_id")]
        proposal_id: String,
        #[serde(rename = "baseRevision", alias = "base_revision")]
        base_revision: u64,
        /// The proposed edit as an operation script.
        script: String,
        /// The script as it was written: before anchors and the lines that
        /// read analysis became the operations in `script`.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        source: String,
        /// Unified diff of the canonical document before and after.
        #[serde(rename = "documentDiff", alias = "document_diff")]
        document_diff: String,
        #[serde(default, skip_serializing_if = "ProposalFreshness::is_current")]
        freshness: ProposalFreshness,
    },
    Accepted {
        #[serde(rename = "proposalId", alias = "proposal_id")]
        proposal_id: String,
        #[serde(rename = "baseRevision", alias = "base_revision")]
        base_revision: u64,
        #[serde(rename = "appliedRevision", alias = "applied_revision")]
        applied_revision: u64,
    },
    Rejected {
        #[serde(rename = "proposalId", alias = "proposal_id")]
        proposal_id: String,
        #[serde(rename = "baseRevision", alias = "base_revision")]
        base_revision: u64,
    },
}
