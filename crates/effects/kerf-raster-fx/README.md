# kerf-raster-fx

The EffectCraft effects Kerf keeps beside FilmCraft's, which FilmCraft has
nothing like: Remove Grain, Dust & Scratches, Rolling Shutter Repair, Optics
Compensation, Detail-preserving Upscale, Bilateral Blur, Radial Zoom Blur,
Shadow/Highlight, Curves, Hue/Saturation, Selective Color, Photo Filter, the
Auto Levels, Contrast, and Color corrections, Key Cleaner, Advanced Spill
Suppressor, Unmult, Simple Choker, Bulge, Motion Tile, Add Grain, and the
Audio Waveform and Spectrum. They draw on `kerf-raster` images, with color
from `kerf-color`. Kerf's renderer runs them through `kerf_fx::kept`, and
native playback and the graph's bake through `kerf_fx::RasterStack`.

They keep EffectCraft's edition (2024), lints, `rustfmt.toml`, and
`clippy.toml`. Where they come from, and their license, is in
[licenses/README.md](../../licenses/README.md).

After a change here:

- If it changes an effect's parameters, regenerate the catalog table with
  `UPDATE_CATALOG=1 cargo test -p kerf-fx`.
