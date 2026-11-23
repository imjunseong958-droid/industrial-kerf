//! What kerfd answers a request it does not carry out with: a [`RuntimeError`], told from the
//! others by its [`ErrorCode`].

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeError {
    pub code: ErrorCode,
    pub message: String,
    pub expected_revision: Option<u64>,
    pub actual_revision: Option<u64>,
}

impl RuntimeError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            expected_revision: None,
            actual_revision: None,
        }
    }

    /// Whether the request did not reach kerfd, or its answer was lost.
    pub fn is_transport(&self) -> bool {
        matches!(
            self.code,
            ErrorCode::IpcUnsupported
                | ErrorCode::IpcConnectError
                | ErrorCode::IpcPermissionDenied
                | ErrorCode::IpcTimeout
                | ErrorCode::IpcError
                | ErrorCode::RuntimeUnavailable
        )
    }

    pub fn conflict(expected: u64, actual: u64) -> Self {
        Self {
            code: ErrorCode::RevisionConflict,
            message: format!("revision conflict: expected {expected}, actual {actual}"),
            expected_revision: Some(expected),
            actual_revision: Some(actual),
        }
    }
}

/// The codes, each with its name on the wire, and the two ways between them: written once, so
/// a code cannot be given one name going out and read by another coming in.
macro_rules! error_codes {
    ($($(#[$note:meta])* $code:ident = $name:literal,)+) => {
        /// Which refusal a [`RuntimeError`] is: what a client decides by, where the message is
        /// what a person reads.
        ///
        /// On the wire a code is its name ([`ErrorCode::as_str`]), such as `REVISION_CONFLICT`.
        /// A kerfd and its clients may be of different builds, so a name stays as it is once
        /// given, and a name this build does not know still reads, as [`ErrorCode::Other`].
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum ErrorCode {
            $($(#[$note])* $code,)+
            /// A code with no name here, as it came: one a kerfd of another build answered
            /// with, or one kerfd passes on as git and GitHub said it
            /// ([`kerf_collab::git::CollabCode`]). Only [`ErrorCode::from_name`] makes one,
            /// so a name this build does know is always its own code, and two codes are
            /// equal just when their names are.
            Other(Unnamed),
        }

        impl ErrorCode {
            /// Its name on the wire.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$code => $name,)+
                    Self::Other(Unnamed(name)) => name,
                }
            }

            // Two codes with one name would make the second arm unreachable.
            #[deny(unreachable_patterns)]
            fn named(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$code),)+
                    _ => None,
                }
            }
        }
    };
}

error_codes! {
    // The request, before anything is asked of the project.
    InvalidJson = "INVALID_JSON",
    InvalidRequest = "INVALID_REQUEST",
    /// An operation this kerfd has no name for: a newer client asked an older kerfd.
    UnknownOperation = "UNKNOWN_OPERATION",
    ProtocolVersionUnsupported = "PROTOCOL_VERSION_UNSUPPORTED",
    RequestTooLarge = "REQUEST_TOO_LARGE",
    InvalidAuthor = "INVALID_AUTHOR",
    AgentProtocolError = "AGENT_PROTOCOL_ERROR",

    // Reaching kerfd, and reading what it answers.
    IpcUnsupported = "IPC_UNSUPPORTED",
    IpcConnectError = "IPC_CONNECT_ERROR",
    IpcPermissionDenied = "IPC_PERMISSION_DENIED",
    IpcUnauthorizedPeer = "IPC_UNAUTHORIZED_PEER",
    IpcTimeout = "IPC_TIMEOUT",
    IpcError = "IPC_ERROR",
    RuntimeUnavailable = "RUNTIME_UNAVAILABLE",
    RuntimeAlreadyRunning = "RUNTIME_ALREADY_RUNNING",
    RuntimeProtocolError = "RUNTIME_PROTOCOL_ERROR",
    /// A failure kerfd gave no code a client can read.
    Unspecified = "RUNTIME_ERROR",

    // An edit.
    RevisionConflict = "REVISION_CONFLICT",
    IdempotencyConflict = "IDEMPOTENCY_CONFLICT",
    InvalidEdit = "INVALID_EDIT",
    /// The script or the operations do not apply to the project: the message says each
    /// diagnostic.
    EditRejected = "EDIT_REJECTED",
    InvalidProject = "INVALID_PROJECT",
    UnknownSequence = "UNKNOWN_SEQUENCE",

    // Media.
    InvalidImport = "INVALID_IMPORT",
    UnknownMedia = "UNKNOWN_MEDIA",
    MediaUnreadable = "MEDIA_UNREADABLE",
    MediaNotDownloaded = "MEDIA_NOT_DOWNLOADED",
    MediaDoesNotFit = "MEDIA_DOES_NOT_FIT",

    // Proposals.
    ProposalNotFound = "PROPOSAL_NOT_FOUND",
    ProposalPending = "PROPOSAL_PENDING",
    ProposalNotPending = "PROPOSAL_NOT_PENDING",
    ProposalAlreadyAccepted = "PROPOSAL_ALREADY_ACCEPTED",
    ProposalRejected = "PROPOSAL_REJECTED",
    ProposalSuperseded = "PROPOSAL_SUPERSEDED",
    ProposalStale = "PROPOSAL_STALE",
    ProposalNotStale = "PROPOSAL_NOT_STALE",
    ProposalBaseGone = "PROPOSAL_BASE_GONE",
    ProposalBaseMismatch = "PROPOSAL_BASE_MISMATCH",
    ProposalNotForwarded = "PROPOSAL_NOT_FORWARDED",

    // Renders.
    InvalidRenderRange = "INVALID_RENDER_RANGE",
    InvalidRenderQuality = "INVALID_RENDER_QUALITY",
    InvalidRenderCodec = "INVALID_RENDER_CODEC",
    InvalidRenderName = "INVALID_RENDER_NAME",
    InvalidRenderOutput = "INVALID_RENDER_OUTPUT",
    InvalidRenderStatus = "INVALID_RENDER_STATUS",
    InvalidRenderProgress = "INVALID_RENDER_PROGRESS",
    RenderOutputExists = "RENDER_OUTPUT_EXISTS",
    RenderOutputBusy = "RENDER_OUTPUT_BUSY",
    RenderNotFound = "RENDER_NOT_FOUND",
    RenderStateConflict = "RENDER_STATE_CONFLICT",
    EngineError = "ENGINE_ERROR",
    Cancelled = "CANCELLED",

    // What kerfd keeps on disk.
    IoError = "IO_ERROR",
    SerializeError = "SERIALIZE_ERROR",
    InvalidCheckpoint = "INVALID_CHECKPOINT",
    CheckpointVersionUnsupported = "CHECKPOINT_VERSION_UNSUPPORTED",
    InvalidJournal = "INVALID_JOURNAL",
    JournalVersionUnsupported = "JOURNAL_VERSION_UNSUPPORTED",
    JournalSequenceError = "JOURNAL_SEQUENCE_ERROR",
    JournalChecksumMismatch = "JOURNAL_CHECKSUM_MISMATCH",
    JournalReplayMismatch = "JOURNAL_REPLAY_MISMATCH",
    ProjectionDiverged = "PROJECTION_DIVERGED",

    // Sharing.
    CollabUnreadableProject = "COLLAB_UNREADABLE_PROJECT",
}

impl ErrorCode {
    /// The code a name stands for. For a name that comes from elsewhere (off the wire, or
    /// from git): a code said here is written as its variant, which the compiler checks.
    pub fn from_name(name: &str) -> Self {
        Self::named(name).unwrap_or_else(|| Self::Other(Unnamed(name.to_owned())))
    }
}

/// The name of a code this build has no code for ([`ErrorCode::Other`]). Nothing outside this
/// module makes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unnamed(String);

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|name| Self::from_name(&name))
    }
}
