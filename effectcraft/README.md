# EffectCraft in Kerf

The EffectCraft crates Kerf draws with: its raster effects and transitions,
keyframes, project model, text, and CPU and GPU renderers. They were copied
from [storytold/effectcraft](https://github.com/storytold/effectcraft) at
`9581b53c3f8945683252af0eb5fac79e0e43aed4` (Release 0.16.0) and are developed
here from then on; Kerf does not follow upstream. Kerf has no use for
EffectCraft's audio, expression-control, paint, 3D-channel, and obsolete
effects, nor for upgrading saved EffectCraft projects, so they are not
here.

They are workspace members, so `cargo check` and `cargo test` cover them. They
keep EffectCraft's edition (2024), lints (in the root `Cargo.toml`),
`rustfmt.toml`, and `clippy.toml`. The layout is upstream's (`crates/`,
`assets/fonts`, `docs/effects.md`), so an upstream fix can still be taken by
hand: in an EffectCraft checkout, `git diff <from> <to> -- crates/<crate>`,
then `git apply --directory=effectcraft` here.

After a change here:

- If it changes a picture, bump `kerf_fx::EFFECTS_VERSION`, so files baked or
  prerendered before it are made again.
- If it changes the effect registry, regenerate the catalog table with
  `UPDATE_CATALOG=1 cargo test -p kerf-fx` and the effects list with
  `UPDATE_DOCS=1 cargo test -p effectcraft-effects --lib effects_doc_is_current`.

The code is MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`); the
bundled fonts are SIL OFL 1.1 (`ATTRIBUTION.md`). The ArtCraft name and logos
are trademarks outside that licence and are not carried.
