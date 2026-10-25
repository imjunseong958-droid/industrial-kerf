# EffectCraft in Kerf

The EffectCraft effects Kerf keeps beside FilmCraft's, which FilmCraft has
nothing like: Remove Grain, Dust & Scratches, Rolling Shutter Repair, Optics
Compensation, Detail-preserving Upscale, Bilateral Blur, Radial Zoom Blur,
Shadow/Highlight, Curves, Hue/Saturation, Selective Color, Photo Filter, the
Auto Levels, Contrast, and Color corrections, Key Cleaner, Advanced Spill
Suppressor, Unmult, Simple Choker, Bulge, Motion Tile, Add Grain, and the
Audio Waveform and Spectrum. `effects` holds them; `raster`, `color`, and
`geom` the pictures, color math, and geometry they run on. Kerf's renderer
runs them through `kerf_fx::kept`, and native playback and the graph's bake
through `kerf_fx::RasterStack`.

They were copied from
[storytold/effectcraft](https://github.com/storytold/effectcraft) at
`9581b53c3f8945683252af0eb5fac79e0e43aed4` (Release 0.16.0) and are developed
here from then on; Kerf does not follow upstream. The rest of EffectCraft
(its other effects and transitions, project model, keyframes, text, tracking,
and CPU and GPU renderers) was taken out when Kerf drew everything with its
own renderer; an id it had is an error that names what to use instead
(`kerf-model/src/retired.tsv`).

They are workspace members, so `cargo check` and `cargo test` cover them. They
keep EffectCraft's edition (2024), lints (in the root `Cargo.toml`),
`rustfmt.toml`, and `clippy.toml`.

After a change here:

- If it changes a picture, bump `kerf_fx::EFFECTS_VERSION`, so files baked
  before it are made again.
- If it changes an effect's parameters, regenerate the catalog table with
  `UPDATE_CATALOG=1 cargo test -p kerf-fx`.

The code is MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). The
ArtCraft name and logos are trademarks outside that licence and are not
carried.
