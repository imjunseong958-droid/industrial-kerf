//! Where a proposal, a render job and the project file stand, as kerfd says them.

use crate::named::named;

named! {
    /// Where a proposal stands.
    pub enum ProposalStatus {
        /// Waiting for a person to accept or reject it.
        Pending = "pending",
        Accepted = "accepted",
        Rejected = "rejected",
    }
}

impl Default for ProposalStatus {
    /// As a status left out reads: none of these.
    fn default() -> Self {
        Self::from_name("")
    }
}

named! {
    /// How `project.kerf` on disk stands to the project kerfd holds.
    pub enum ProjectionStatus {
        /// The file says the project kerfd holds.
        Current = "current",
        /// There is no file.
        Missing = "missing",
        /// The file says something else: it was changed outside Kerf.
        Diverged = "diverged",
    }
}

named! {
    /// Where a render job stands.
    pub enum RenderStatus {
        /// Waiting for a worker to take it.
        Queued = "queued",
        Running = "running",
        Completed = "completed",
        Failed = "failed",
        Cancelled = "cancelled",
    }
}

impl Default for RenderStatus {
    /// As a status left out reads: none of these.
    fn default() -> Self {
        Self::from_name("")
    }
}

impl RenderStatus {
    /// Whether a job that stands so has ended.
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Whether a job that stands so is still to end: waiting, or being rendered.
    pub fn is_under_way(&self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}
