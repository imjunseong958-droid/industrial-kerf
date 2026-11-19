//! FilmCraft text engine (layer L1).
//!
//! - [`fonts`]: font database — bundled OFL fonts (Inter, JetBrains Mono, Noto Serif), the optional
//!   craft-fonts ([`fonts::CRAFT_FONTS`], Japanese; empty unless built with `CRAFT_FONTS_DIR`) plus faces
//!   discovered by [`fonts::FontSource`]s (system font folders on native platforms; nothing on the
//!   web unless the host registers font data), family/style resolution (by family, face name or
//!   weight) with synthetic bold/italic, and per-cluster fallback to bundled, craft-fonts and
//!   installed faces, colour emoji faces first for emoji.
//! - [`layout`]: shaping (harfrust: kerning, ligatures, complex scripts), bidi, line breaking,
//!   paragraph layout, per-character style runs, carets and hit testing; cached.
//! - [`raster`]: our own anti-aliased path rasteriser (linear coverage, so compositing in linear
//!   light is correct) and vector shapes.
//! - [`render`]: glyph outlines and the glyph-mask cache (sub-pixel positioned), drawing layouts
//!   under any affine transform.
//! - [`mask`]: strokes (outer/centre/inner via a Euclidean distance transform), blur, offsets.
//! - [`color`]: colour glyphs (emoji): `COLR` v0/v1 layers and `CBDT`/`sbix` PNG bitmaps.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod color;
pub mod fonts;
pub mod layout;
pub mod mask;
pub mod raster;
pub mod render;
pub mod sfnt;

pub use fonts::{FaceId, Resolved, families, resolve};
pub use layout::{Align, Caps, Glyph, Layout, Line, ParagraphStyle, StyleRun, TextStyle, layout, layout_rich, measure};
pub use raster::{Mask, Path, Xform};

#[cfg(test)]
mod perf {
    use super::*;

    /// A three-line 1080p title: layout + rasterisation must stay far below a frame (≈ 41 ms at
    /// 24 fps). Cold = first time (shaping + glyph rasterisation); warm = cached.
    #[test]
    fn three_line_title_is_fast() {
        let text = "FILMCRAFT PRESENTS\nA Night Drive Through the City\nDirected by Nobody in Particular";
        let st = TextStyle { size: 96.0, style: "Bold".into(), ..Default::default() };
        let ps = ParagraphStyle { align: Align::Center, ..Default::default() };
        let t0 = std::time::Instant::now();
        let l = layout::layout_uncached(text, &st, &ps);
        let (m, _, _) = render::rasterize(&l, &render::at(960.0, 400.0));
        let cold = t0.elapsed();
        let _ = layout(text, &st, &ps);
        // The best of a few: the other tests of this binary run beside it, scanning and reading
        // every installed font, and one run can wait on them, for the font database's lock while
        // the system's faces go in or for a core, which is no time of this path's own.
        let warm = (0..5)
            .map(|_| {
                let t1 = std::time::Instant::now();
                let l2 = layout(text, &st, &ps);
                let (m2, _, _) = render::rasterize(&l2, &render::at(960.0, 400.0));
                let warm = t1.elapsed();
                assert_eq!(m.a.len(), m2.a.len());
                warm
            })
            .min()
            .unwrap_or_default();
        eprintln!("3-line 1080p title: cold {cold:?}, warm {warm:?} (mask {}x{})", m.w, m.h);
        // generous bound for debug builds on busy CI machines
        assert!(warm.as_millis() < 40, "warm {warm:?}");
    }
}
