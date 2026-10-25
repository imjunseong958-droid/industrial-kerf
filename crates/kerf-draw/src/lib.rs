//! Kerf's drawing.
//!
//! Pictures are worked on as premultiplied RGBA in linear light, in
//! floating point ([`Image`]), as FilmCraft's renderer works on them: media
//! decode to sRGB-encoded bytes, which [`Image::from_rgba8`] takes into
//! linear light, and a finished frame goes back to bytes with
//! [`Image::to_rgba8_over_black`]. A dissolve or a blur in linear light
//! mixes light as a camera would; the blend modes other than normal, and
//! color corrections written for encoded values, convert around their
//! arithmetic.
//!
//! - [`Image`]: sampling, resampling through an [`Affine`], mixing.
//! - [`blend`]: FilmCraft's 27 blend modes.
//! - [`mask`]: a clip's masks as coverage, from the distance to each
//!   outline.
//! - [`blur`]: Gaussian blur by box passes.
//! - [`correct`]: `color.basic`, with the arithmetic of the graph's
//!   filters.
//! - [`text`]: titles and captions, laid out and drawn by FilmCraft's text
//!   engine.
//! - [`wipe`]: the coverage of a linear wipe.

pub mod blend;
pub mod blur;
pub mod color;
pub mod correct;
pub mod image;
pub mod mask;
pub mod text;
pub mod wipe;

pub use blend::Blend;
pub use filmcraft_geom::{Affine, Rect, Vec2};
pub use image::Image;
