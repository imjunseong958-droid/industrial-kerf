# FilmCraft in Kerf

The FilmCraft crates Kerf draws with: geometry, color science, time, and the
text engine with its bundled fonts. They were copied from
[storytold/filmcraft](https://github.com/storytold/filmcraft) at
`0f9966189daf46dcdc4bcb1fb73441bac80076be` and are developed here from then
on; Kerf does not follow upstream. FilmCraft's renderer, transitions, and
video effects are carried into Kerf's own crates in Kerf's terms, as
[docs/looks.md](../docs/looks.md) plans: the drawing into `kerf-draw`, the
transitions into `kerf-fx/src/transitions`, and the video effects with the
effect definitions they read into `kerf-fx/src/filmcraft`.

They are workspace members, so `cargo check` and `cargo test` cover them. They
keep FilmCraft's edition (2024), lints (in the root `Cargo.toml`),
`rustfmt.toml`, and `clippy.toml`. The layout is upstream's (`crates/`,
`assets/fonts`), so an upstream fix can still be taken by hand: in a
FilmCraft checkout, `git diff <from> <to> -- crates/<crate>`, then
`git apply --directory=filmcraft` here.

The code is MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`); the
bundled fonts are SIL OFL 1.1 (`ATTRIBUTION.md`). The ArtCraft name and logos
are trademarks outside that licence and are not carried.
