//! FilmCraft text engine (layer L1).
//!
//! - [`fonts`]: font database — bundled OFL fonts (Inter, JetBrains Mono, Noto Serif), the optional
//!   craft-fonts ([`fonts::CRAFT_FONTS`], Japanese; empty unless built with `CRAFT_FONTS_DIR`) plus faces
//!   discovered by [`fonts::FontSource`]s (system font folders on native platforms; nothing on the
//!   web unless the host registers font data), family/style resolution (by family, face name or
//!   weight) with synthetic bold/italic, and per-cluster fallback to bundled, craft-fonts and
//!   installed faces, colour emoji faces first for emoji.
//! - [`mod@layout`]: shaping (harfrust: kerning, ligatures, complex scripts), bidi, line breaking,
//!   paragraph layout, per-character style runs, carets and hit testing; cached.
//! - [`raster`]: our own anti-aliased path rasteriser (linear coverage, so compositing in linear
//!   light is correct) and vector shapes.
//! - [`render`]: glyph outlines and the glyph-mask cache (sub-pixel positioned), drawing layouts
//!   under any affine transform.
//! - [`mask`]: strokes (outer/centre/inner via a Euclidean distance transform), blur, offsets.
//! - [`color`]: colour glyphs (emoji): `COLR` v0/v1 layers and `CBDT`/`sbix` PNG bitmaps.

#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unimplemented,
        clippy::todo,
        clippy::unreachable
    )
)]
#![expect(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "carried from FilmCraft: loops index by position, shaping takes many inputs, caches nest types"
)]

pub mod color;
pub mod fonts;
pub mod layout;
pub mod mask;
pub mod raster;
pub mod render;
pub mod sfnt;

pub use fonts::{families, resolve, FaceId, Resolved};
pub use layout::{
    layout, layout_rich, measure, Align, Caps, Glyph, Layout, Line, ParagraphStyle, StyleRun,
    TextStyle,
};
pub use raster::{Mask, Path, Xform};
