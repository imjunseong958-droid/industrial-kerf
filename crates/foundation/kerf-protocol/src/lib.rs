//! What kerfd and its clients say to each other.
//!
//! kerfd is the only owner of a project; the editor, the terminal agent and the MCP server
//! reach it through the same requests and read the same answers. This crate is those words
//! and nothing that carries them out: a client that only needs to say or read them takes
//! this crate, and need not take kerfd with it.
//!
//! A [`RuntimeRequest`] asks, in a [`RequestEnvelope`]; a [`RuntimeResponse`] or a
//! [`RuntimeError`] answers. What a client is shown of the project is a [`StateSnapshot`],
//! with its [`TimelineView`] and its proposals; what an editor's gesture changes is an
//! [`EditOp`]. The values kerfd writes as names ([`ErrorCode`], the statuses, a render's
//! quality and codec, an author's kind) are enums that keep a name this build does not know.

mod author;
mod edit;
mod error;
mod named;
mod render;
mod request;
mod state;
mod status;
mod view;

pub use author::{Author, AuthorKind};
pub use edit::{EditOp, GeneratorSpec, MaskOutline, RateSpec, TrimSide};
pub use error::{ErrorCode, RuntimeError};
pub use named::Unnamed;
pub use render::{RenderJob, RenderRange, StartRenderOptions, MAX_RENDER_RANGE_SECONDS};
pub use request::{validate_actor, RequestEnvelope, RuntimeRequest, RuntimeResponse};
pub use state::{
    PendingProposal, ProposalEntry, ProposalFreshness, ProposalVersion, StateSnapshot,
    MAX_PROPOSALS, SOURCE_VERSION,
};
pub use status::{ProjectionStatus, ProposalStatus, RenderCodec, RenderQuality, RenderStatus};
pub use view::{
    TimelineView, ViewClip, ViewEffect, ViewMarker, ViewMask, ViewOutline, ViewOutlineKey,
    ViewSequence, ViewSource, ViewTake, ViewTrack, ViewTrackKind, ViewTransition,
};
