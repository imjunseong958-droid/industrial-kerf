//! The Kerf editing kernel.
//!
//! Exact time, the typed project model, the feature catalog, validation, and
//! the NLE edit operations. Every surface (editor, agent, MCP, render) reads
//! and writes projects through this crate; text and wire formats live outside.

pub mod catalog;
pub mod diag;
pub mod id;
pub mod look;
pub mod model;
pub mod ops;
pub mod time;
pub mod validate;
pub mod value;

pub use catalog::{Area, Catalog, Domain, Engine, FxDef, FxKind, ParamDef, ParamType};
pub use diag::{Diagnostic, Severity, Span};
pub use id::{Id, IdGen, InvalidId};
pub use model::{
    track_role, Align, AudioInfo, Clip, ClipAt, Content, Effect, Marker, MarkerColor, Mask,
    MaskMode, MaskShape, Media, Params, Project, Sequence, Track, TrackKind, Transition, VideoInfo,
    TRACK_ROLES,
};
pub use ops::{
    apply, ClipChange, Edge, MarkerChange, NewClip, Op, Rejected, Report, SequenceChange,
    TrackChange, TrackRoleChange,
};
pub use time::{Rate, Ratio, Time, TimeError};
pub use validate::validate;
pub use value::{Interp, Keyframe, Param, Rgba, Value};
