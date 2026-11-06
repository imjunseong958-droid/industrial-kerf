# Licenses

Kerf is MIT licensed (`LICENSE` at the root). This folder holds
the licenses and notices of what Kerf carries from elsewhere: code from
ArtCraft's EffectCraft and FilmCraft, and the fonts its text engine bundles.
`NOTICE` gathers their notices.

## EffectCraft and FilmCraft

Kerf copied this code from
[storytold/effectcraft](https://github.com/storytold/effectcraft) at
`9581b53c3f8945683252af0eb5fac79e0e43aed4` (Release 0.16.0) and
[storytold/filmcraft](https://github.com/storytold/filmcraft) at
`0f9966189daf46dcdc4bcb1fb73441bac80076be`. It is developed here from then
on, and Kerf does not follow upstream.

| Where | What it carries |
|---|---|
| `crates/effects/kerf-raster-fx` | The 24 EffectCraft effects Kerf keeps, with the tracking and plane code they run on |
| `crates/render/kerf-raster` | EffectCraft's raster images, warps, and blurs, the feature tracker, and optical flow |
| `crates/image/kerf-image/src/fill.rs` | EffectCraft's hole filling |
| `crates/vfx/kerf-track/src/planar.rs` | EffectCraft's homography fit and RANSAC |
| `crates/vfx/kerf-track/src/roto` | EffectCraft's graph cut, colour models, and matte propagation |
| `crates/vfx/kerf-segment` | EffectCraft's model interface and registry, PyTorch reader, and MobileSAM in Rust. MobileSAM's weights are Apache-2.0 and never bundled |
| `crates/color/kerf-scopes` | FilmCraft's scope summaries, reduced to what an agent reads |
| `crates/foundation/kerf-model/looks/grade-*.kerflook` | FilmCraft's Lumetri presets, as looks |
| `crates/foundation/kerf-color` | FilmCraft's color science and EffectCraft's blend modes |
| `crates/foundation/kerf-geom` | FilmCraft's and EffectCraft's 2D geometry |
| `crates/foundation/kerf-text` | FilmCraft's text engine |
| `crates/render/kerf-draw` | FilmCraft's renderer, in Kerf's terms |
| `crates/foundation/kerf-text/testdata` | Nothing carried: the small variable font the tests set axes on is Kerf's own, written by the script beside it |
| `crates/engine/kerf-compose/src/draw/focus.rs` | EffectCraft's circle of confusion and progressive blur for depth of field (`render/three_d/bokeh.rs`) |
| `crates/motion/kerf-motion/src/outline.rs` | EffectCraft's path offsetting and how it uses the sweep (`crates/path`), on polygons rather than curves |
| `crates/render/kerf-gpu/src/picture.wgsl` | FilmCraft's blend modes and dissolve pattern in WGSL |
| `crates/effects/kerf-fx/src/filmcraft`, `crates/effects/kerf-fx/src/transitions` | FilmCraft's video effects and transitions, with its GPU effect kernels (`filmcraft/gpu.rs`, `fx.wgsl`) |

That code is MIT OR Apache-2.0, at your option: `LICENSE-MIT-ArtCraft` and
`LICENSE-APACHE-2.0`. The ArtCraft name, wordmark, and logos are trademarks
outside that license and are not carried.

## Fonts

`crates/foundation/kerf-text/fonts` holds these fonts, unmodified. The text engine
embeds them with `include_bytes!`.

| Font | Author | Source | License |
|---|---|---|---|
| `Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf`, `Inter-Bold.ttf`, `Inter-Italic.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`OFL-Inter.txt`) |
| `NotoSerif-Regular.ttf` | The Noto Project Authors | https://github.com/notofonts/latin-greek-cyrillic | OFL-1.1 (`OFL-NotoSerif.txt`) |
| `JetBrainsMono-Regular.ttf` | The JetBrains Mono Project Authors | https://github.com/JetBrains/JetBrainsMono | OFL-1.1 (`OFL-JetBrainsMono.txt`) |
