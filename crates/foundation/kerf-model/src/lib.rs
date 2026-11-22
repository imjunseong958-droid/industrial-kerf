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
pub mod making;
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
pub use kerf_color::ColorSpace;
pub use making::Pass;
pub use model::{
    track_role, Align, AudioInfo, Bus, Clip, ClipAt, Content, Effect, Marker, MarkerColor, Mask,
    MaskMode, MaskShape, Media, OutlineKey, Params, Project, Retime, Sequence, Synced, Track,
    TrackKind, Transition, VideoInfo, TRACK_ROLES,
};
pub use ops::{
    apply, BusChange, BusFx, ClipChange, Edge, MarkerChange, NewClip, Op, Rejected, Report,
    SequenceChange, Steps, TrackChange, TrackRoleChange,
};
pub use path::{parse_moves, parse_path_points, parse_points, MotionPath, PathPoint};
pub use speed::{Ramp, RampKey, Speed};
pub use time::{Rate, Ratio, Time, TimeError};
pub use validate::validate;

/// The effect that gives a `gen=shape` clip another outline. It draws no pass of its own: the shape
/// reads it where it draws.
pub const SHAPE_OUTLINE: &str = "shape.outline";

/// The generator that draws nothing and holds a number over time for a model: a recipe's
/// `generate … signal=<clip>` hands it what [`SIGNAL_VALUE`] comes to on each frame.
pub const SIGNAL: &str = "signal";

/// The parameter of a [`SIGNAL`] clip that is the signal: 0 to 1, keyframed.
pub const SIGNAL_VALUE: &str = "value";

/// The axes of a variable font a title can set, by the tags fonts name them with.
pub const TITLE_AXES: [[u8; 4]; 5] = [*b"wght", *b"wdth", *b"slnt", *b"opsz", *b"ital"];

/// The style a title's face is asked for by, from its `style` and `weight`: the style name, with
/// the weight in front of it where one is set (above 0), which then holds over a weight the name
/// gives ("900 italic", "300 Black").
pub fn title_face(style: &str, weight: f64) -> String {
    let style = style.trim();
    if weight > 0.0 && weight.is_finite() {
        format!("{} {style}", weight.round() as u32)
            .trim_end()
            .to_owned()
    } else {
        style.to_owned()
    }
}
pub use value::{Bezier, Interp, Keyframe, Param, Rgba, Value};
