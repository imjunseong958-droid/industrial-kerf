//! Where a proposal, a render job and the project file stand, and what a render is asked
//! for, as kerfd says them.

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

named! {
    /// What a render is made for, which says how it is encoded where no codec does.
    pub enum RenderQuality {
        /// A draft from proxies.
        Preview = "preview",
        Full = "full",
        /// A full-size review copy: H.264 at 12 Mb/s.
        Review = "review",
        /// The finished file: ProRes 422 HQ unless a codec says otherwise.
        Master = "master",
    }
}

impl Default for RenderQuality {
    /// As a quality left out reads: none of these.
    fn default() -> Self {
        Self::from_name("")
    }
}

named! {
    /// The codec a render is asked for, or was made with.
    pub enum RenderCodec {
        H264 = "h264",
        Hevc = "hevc",
        HevcHdr = "hevc_hdr",
        /// ProRes 422 HQ, in QuickTime with 24-bit PCM.
        ProresHq = "prores_hq",
    }
}

impl Default for RenderCodec {
    /// As a codec left out reads: none of these.
    fn default() -> Self {
        Self::from_name("")
    }
}
