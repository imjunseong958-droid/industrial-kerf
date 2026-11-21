# kerf-text

Kerf's text engine, carried from FilmCraft (depends on `skrifa`, `harfrust`, `unicode-bidi`,
`unicode-linebreak`, `unicode-segmentation` and `png`). It draws graphic-clip text and shapes, caption burn-in
and the Timecode / Clip Name effects. Pure Rust; it builds for `wasm32-unknown-unknown`.

| Module | What |
|---|---|
| `fonts` | Font database. A `TextStyle`'s `axes` set a variable font's axes, by tag, for shaping and for outlines (`instance`); a face without them ignores them, and at their defaults it draws as it comes. A face's `features()` are the OpenType features it has, by tag. Bundled faces (always present, also on the web): Inter Regular / Medium / SemiBold / Bold / Italic, JetBrains Mono Regular, Noto Serif Regular — all SIL OFL 1.1, unmodified, in `fonts/` (see `licenses/README.md` at the repository root). More faces come from `FontSource`s: `DirectorySource::system()` scans the platform font folders on first need (only the table directory, `name` and `OS/2` are read, via `sfnt`; colour faces with `CBDT` or `sbix` bitmaps and no outlines are listed too); on wasm the scan finds nothing and hosts can call `add_font_data`. `resolve(family, style)` picks a face, synthesises bold/italic when the family lacks them, and falls back to Inter (flagged `missing`). A style names a weight by word or number (`Black`, `900 Italic`, `W6`; `style_weight`); a family is also found by a face's full, PostScript or legacy name (`Inter Black`, `Inter-Black`) or as a family followed by style words (`NanumSquare Bold`; `family_named`), and the system is scanned when the bundled faces have no face of the weight and slant asked for. In a variable face a style's weight and italic set `wght` and `ital` (`style_axes`). `face_for_cluster` finds a face for a grapheme cluster the chosen face lacks (joiners and variation selectors aside): emoji (`wants_emoji`: `Emoji_Presentation` characters, U+FE0F, skin tones) go to colour faces first; then the bundled faces, the optional craft-fonts faces (`CRAFT_FONTS`, embedded by `build.rs` when built with `CRAFT_FONTS_DIR`; Mincho for serif text, Gothic otherwise, nearest weight), then installed faces family by family (Korean, Japanese and Chinese Gothics or Myeongjo/Mincho first, placeholder fonts such as LastResort never), each family's face nearest the text's weight. Which characters a face has (`Face::covers`) is read from its `cmap` alone, so looking through installed fonts does not load them. |
| `layout` | Paragraph layout: per-character font fallback → bidi levels (UAX #9) → items (runs of one face, direction and size) shaped with harfrust (OpenType kerning, ligatures, marks, complex scripts, and the features a `TextStyle`'s `features` turn on or off by tag, which hold over `kerning` and `ligatures` and over what the font has on by itself) → tracking → line breaking at UAX #14 opportunities (area text) → visual reordering per line → alignment (left / centre / right / justify), leading (added to the natural line height), baseline shift, all caps / synthesised small caps (78 %), underline. Every character boundary gets a caret stop (ligatures are split evenly), so editors can place carets, hit-test and draw selections. Layouts are cached by text + style (512 entries, LRU). |
| `layout_rich` | Per-character style runs (`StyleRun`: a byte range with its own `TextStyle`): runs split shaping items (font, size, tracking, caps, faux styles), baseline shift and underline apply per run, a line holding a larger run is pushed down from the line above (baseline distance = max(base line height, previous descent + this line's ascent + leading)), and every glyph / underline carries its style index so renderers can colour runs (`render::draw_run`). |
| `raster` | Our anti-aliased rasteriser (non-zero winding, 5 sub-scanlines, exact horizontal coverage, active-edge list), contour paths with transforms, rounded rectangles, ellipses, polygons and outline emboldening. Coverage is linear, so compositing in linear light is correct. |
| `render` | Glyph outlines (cached per face + glyph) and glyph masks (cached per size in ⅛ px, ¼-px horizontal phase, synthetic style). Axis-aligned uniform transforms use the mask cache with sub-pixel x positioning; rotated / skewed / non-uniform transforms fill the transformed outlines of the whole run in one pass. Synthetic bold grows outlines by 2.5 % of the size; synthetic italic slants by 12°. |
| `color` | Colour glyphs (emoji): `COLR` v0 layers and v1 paint graphs (solid, linear, radial and sweep gradients, transforms, clips, Porter–Duff and blend composites) filled as outlines with the font's `CPAL` palette and the text's colour for index 0xFFFF; `CBDT` / `sbix` PNG (and BGRA, mask) bitmaps from the nearest strike, resampled bilinear from a halved copy below half size, with a one-pixel edge. Any affine transform. Colours are sRGB-encoded and premultiplied. `render::rasterize` adds a colour glyph's alpha to the coverage (so strokes and shadows go round it), and `render::rasterize_color` / `draw_color` give its colours in the same place, to draw over the fill. Tested with the small colour fonts `testdata/make_color_fonts.py` writes. |
| `mask` | Outer / centre / inner strokes from an exact Euclidean distance transform (Felzenszwalb–Huttenlocher) with sub-pixel refinement, a three-pass box blur (≈ Gaussian) for shadows. |

## Coordinates

Pixels, y down. Point text: the origin is the alignment point on the first baseline (left edge,
centre or right edge). Area text (`ParagraphStyle::width`): the origin is the box's top-left.

## Performance

A three-line, centred, 96 px bold title at 1080p, release build on an M-series Mac: about
1.6 ms cold (shaping, outline extraction, glyph rasterisation) and 0.15 ms warm (layout cache +
glyph cache), measured by hand; no test holds it to a time.

## Not yet

Hinting, the hue / saturation / colour / luminosity composites of COLR v1 (drawn as source over),
and hyphenation. Words longer than an area-text line overflow instead of breaking mid-word.
