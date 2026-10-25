//! The video effects carried from FilmCraft (`crates/render/src/{effects,gpufx,vfx}` and the
//! definitions in `crates/project/src/effect`), which Kerf's renderer draws.
//!
//! [`catalog`] lists them under Kerf's ids and writes their catalog table; [`instance`] reads a
//! clip's effect back into the [`project::EffectInstance`] FilmCraft's effects run from, with
//! its parameters evaluated by Kerf at the time drawn; [`vfx::FxEnv`] gives them the clip's
//! other frames and the sequence's other tracks. Their pictures are premultiplied linear RGBA
//! ([`kerf_draw::Image`]).

pub mod catalog;
pub mod effects;
pub mod gpufx;
pub mod graphics;
pub mod instance;
pub mod luts;
pub mod project;
pub mod track;
pub mod vfx;

/// Whether long analyses (the Warp Stabilizer's) should stop early; Kerf runs them to the end.
pub(crate) fn cancelled() -> bool {
    false
}
