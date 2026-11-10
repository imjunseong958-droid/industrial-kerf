//! The Kerf editing kernel.
//!
//! Exact time, the typed project model, the feature catalog, validation, and
//! the NLE edit operations. Every surface (editor, agent, MCP, render) reads
//! and writes projects through this crate; text and wire formats live outside.

pub mod catalog;
pub mod diag;
pub mod expr;
mod features;
pub mod id;
pub mod look;
pub mod model;
pub mod ops;
mod path;
pub mod speed;
pub mod time;
pub mod validate;
pub mod value;

pub use catalog::{Area, Catalog, Domain, Engine, FxDef, FxKind, ParamDef, ParamType};
pub use diag::{Diagnostic, Severity, Span};
pub use features::parse_features;
pub use id::{Id, IdGen, InvalidId};
pub use model::{
    Retime,
    track_role, Align, AudioInfo, Clip, ClipAt, Content, Effect, Marker, MarkerColor, Mask,
    Bus, MaskMode, MaskShape, Media, OutlineKey, Params, Project, Sequence, Track, TrackKind, Transition, VideoInfo,
    TRACK_ROLES,
};
pub use kerf_color::ColorSpace;
pub use path::{parse_moves, parse_path_points, parse_points, MotionPath, PathPoint};
pub use ops::{
    apply, BusChange, BusFx, ClipChange, Edge, MarkerChange, NewClip, Op, Rejected, Report, SequenceChange,
    TrackChange, TrackRoleChange,
};
pub use speed::{Ramp, RampKey, Speed};
pub use time::{Rate, Ratio, Time, TimeError};
pub use validate::validate;

/// The effect that gives a `gen=shape` clip another outline. It draws no pass of its own: the shape
/// reads it where it draws.
pub const SHAPE_OUTLINE: &str = "shape.outline";

/// The axes of a variable font a title can set, by the tags fonts name them with.
pub const TITLE_AXES: [[u8; 4]; 5] = [*b"wght", *b"wdth", *b"slnt", *b"opsz", *b"ital"];
pub use value::{Bezier, Interp, Keyframe, Param, Rgba, Value};
