//! Kerf's image tools: the little of a photo editor that video work has
//! always needed, on a clip's picture rather than on a separate document.
//! Kerf is an editor, so this area stays small: see `docs/architecture.md`.
//!
//! [`EFFECTS`] are this area's effects, which the compositor runs through
//! [`kerf_draw::effect::PictureEffect`]; their definitions are the catalog's
//! `image_effects.tsv`.

mod effects;
pub mod fill;
pub mod stroke;
pub mod warp;

pub use effects::EFFECTS;
