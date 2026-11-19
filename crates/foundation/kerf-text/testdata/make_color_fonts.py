#!/usr/bin/env python3
"""Writes KerfTestColr.ttf and KerfTestBitmap.ttf, the small colour fonts the tests draw emoji with.

Linux CI has no colour font, and no font has the private-use characters these map, so what a test
finds in a picture or in a fallback can be checked against a number. Kerf's own work under the
repository's license; the table helpers come from make_test_font.py.

KerfTestColr.ttf ("Kerf Test COLR"), 1000 units to the em, every glyph 800 wide:
    U+10FFFC  COLR v0: a box from (100, 0) to (700, 600) in palette red, and over it an inner box
              from (300, 200) to (500, 400) in the text's own colour (palette index 0xFFFF).
    U+10FFFB  COLR v1: the box filled with a linear gradient from red at x 100 to blue at x 700.
    Both glyphs also have the box as their outline, as COLR fonts do.

KerfTestBitmap.ttf ("Kerf Test Bitmap"), 1000 units to the em, no outlines (like Noto Color Emoji):
    U+10FFFD and U+1F480  CBDT: an 8 x 8 PNG at 8 pixels to the em, red with a blue lower-right
              quarter, its top on the ascender and as wide as its advance (one em).

Run it from this folder with `python3 make_color_fonts.py`.
"""
import struct
import zlib

from make_test_font import be, f2dot14, os2, post, sfnt

RED, BLUE = (255, 0, 0, 255), (0, 0, 255, 255)


def head():
    return be(
        ("L", 0x00010000), ("L", 0x00010000), ("L", 0), ("L", 0x5F0F3CF5), ("H", 0x000B), ("H", 1000),
        ("q", 0), ("q", 0), ("h", 0), ("h", 0), ("h", 800), ("h", 800), ("H", 0), ("H", 8), ("h", 2),
        ("h", 1), ("h", 0),
    )


def hhea(glyphs):
    return be(
        ("L", 0x00010000), ("h", 800), ("h", -200), ("h", 0), ("H", 1000), ("h", 0), ("h", 0), ("h", 800),
        ("h", 1), ("h", 0), ("h", 0), ("h", 0), ("h", 0), ("h", 0), ("h", 0), ("h", 0), ("H", glyphs),
    )


def hmtx(metrics):
    """Each glyph's advance and left side bearing, which is where its outline starts."""
    return b"".join(be(("H", advance), ("h", lsb)) for advance, lsb in metrics)


def cmap(groups):
    subtable = be(("H", 12), ("H", 0), ("L", 16 + 12 * len(groups)), ("L", 0), ("L", len(groups)))
    for group in sorted(groups):
        subtable += struct.pack(">LLL", *group)
    return be(("H", 0), ("H", 1), ("H", 3), ("H", 10), ("L", 12)) + subtable


def name(family, postscript):
    strings = {1: family, 2: "Regular", 4: family, 6: postscript}
    records = b""
    storage = b""
    for id_, text in strings.items():
        encoded = text.encode("utf-16-be")
        records += struct.pack(">HHHHHH", 3, 1, 0x0409, id_, len(encoded), len(storage))
        storage += encoded
    return be(("H", 0), ("H", len(strings)), ("H", 6 + len(records))) + records + storage


def rectangle(x0, y0, x1, y1):
    """A simple glyph: one clockwise rectangle."""
    glyph = be(("h", 1), ("h", x0), ("h", y0), ("h", x1), ("h", y1), ("H", 3), ("H", 0))
    glyph += bytes([1, 1, 1, 1])
    # x deltas then y deltas, from (x0, y0) up, across and down: a clockwise rectangle.
    xs = (x0, 0, x1 - x0, 0)
    ys = (y0, y1 - y0, 0, y0 - y1)
    for delta in xs + ys:
        glyph += struct.pack(">h", delta)
    return glyph + b"\0" * (-len(glyph) % 4)


def uint24(value):
    return struct.pack(">I", value)[1:]


def colr():
    # v0: glyph 4 is the box in red, then the inner box in the foreground colour.
    base_v0 = be(("H", 4), ("H", 0), ("H", 2))
    layers = be(("H", 2), ("H", 0), ("H", 3), ("H", 0xFFFF))
    # v1: glyph 5 is PaintGlyph(box) over PaintLinearGradient over a two-stop ColorLine.
    color_line = bytes([0]) + be(("H", 2), ("h", f2dot14(0.0)), ("H", 0), ("h", f2dot14(1.0)),
                                 ("h", f2dot14(1.0)), ("H", 1), ("h", f2dot14(1.0)))
    gradient = bytes([4]) + uint24(16) + be(("h", 100), ("h", 0), ("h", 700), ("h", 0), ("h", 100), ("h", 600))
    paint_glyph = bytes([10]) + uint24(6) + be(("H", 2))
    base_list = be(("L", 1), ("H", 5), ("L", 10)) + paint_glyph + gradient + color_line
    header_size = 34
    records_at = header_size
    layers_at = records_at + len(base_v0)
    list_at = layers_at + len(layers)
    header = be(("H", 1), ("H", 1), ("L", records_at), ("L", layers_at), ("H", 2), ("L", list_at), ("L", 0),
                ("L", 0), ("L", 0), ("L", 0))
    assert len(header) == header_size
    return header + base_v0 + layers + base_list


def cpal():
    colors = [RED, BLUE]
    records = b"".join(bytes([b, g, r, a]) for r, g, b, a in colors)
    return be(("H", 0), ("H", len(colors)), ("H", 1), ("H", len(colors)), ("L", 14), ("H", 0)) + records


def colr_font():
    glyphs = [b"", b"", rectangle(100, 0, 700, 600), rectangle(300, 200, 500, 400), rectangle(100, 0, 700, 600),
              rectangle(100, 0, 700, 600)]
    outlines = b"".join(glyphs)
    offsets = [0]
    for glyph in glyphs:
        offsets.append(offsets[-1] + len(glyph))
    return sfnt(0x00010000, {
        b"COLR": colr(),
        b"CPAL": cpal(),
        b"OS/2": os2(),
        b"cmap": cmap([(0x20, 0x20, 1), (0x10FFFB, 0x10FFFB, 5), (0x10FFFC, 0x10FFFC, 4)]),
        b"glyf": outlines,
        b"head": head(),
        b"hhea": hhea(len(glyphs)),
        b"hmtx": hmtx([(800, 0), (800, 0), (800, 100), (800, 300), (800, 100), (800, 100)]),
        b"loca": b"".join(struct.pack(">L", offset) for offset in offsets),
        b"maxp": be(("L", 0x00010000), ("H", len(glyphs)), ("H", 4), ("H", 1)) + b"\0" * 2 * 11,
        b"name": name("Kerf Test COLR", "KerfTestCOLR-Regular"),
        b"post": post(),
    })


def png(width, height, pixel):
    """An RGBA PNG whose pixel (x, y) is pixel(x, y)."""
    rows = b"".join(b"\0" + b"".join(bytes(pixel(x, y)) for x in range(width)) for y in range(height))

    def chunk(kind, data):
        return struct.pack(">L", len(data)) + kind + data + struct.pack(">L", zlib.crc32(kind + data) & 0xFFFFFFFF)

    header = struct.pack(">LLBBBBB", width, height, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def line_metrics():
    # ascender, descender, widthMax, caret slope (2), caret offset, minOriginSB, minAdvanceSB,
    # maxBeforeBL, minAfterBL, two pads.
    return struct.pack(">bbBbbbbbbbbb", 8, -2, 8, 1, 0, 0, 0, 0, 8, -2, 0, 0)


def bitmap_tables():
    image = png(8, 8, lambda x, y: BLUE if x >= 4 and y >= 4 else RED)
    # Format 17: small metrics (height, width, bearingX, bearingY, advance), then the PNG.
    glyph = struct.pack(">BBbbB", 8, 8, 0, 8, 8) + struct.pack(">L", len(image)) + image
    cbdt = be(("H", 3), ("H", 0)) + glyph
    # One index subtable for glyph 2, format 1 with image format 17, its data 4 bytes into CBDT.
    subtable = be(("H", 1), ("H", 17), ("L", 4), ("L", 0), ("L", len(glyph)))
    array = be(("H", 2), ("H", 2), ("L", 8))
    size_at = 8
    array_at = size_at + 48
    size = (
        be(("L", array_at), ("L", len(array) + len(subtable)), ("L", 1), ("L", 0))
        + line_metrics() + line_metrics()
        + be(("H", 2), ("H", 2)) + bytes([8, 8, 32, 1])
    )
    assert len(size) == 48
    cblc = be(("H", 3), ("H", 0), ("L", 1)) + size + array + subtable
    return cbdt, cblc


def bitmap_font():
    cbdt, cblc = bitmap_tables()
    return sfnt(0x00010000, {
        b"CBDT": cbdt,
        b"CBLC": cblc,
        b"OS/2": os2(),
        b"cmap": cmap([(0x20, 0x20, 1), (0x1F480, 0x1F480, 2), (0x10FFFD, 0x10FFFD, 2)]),
        b"head": head(),
        b"hhea": hhea(3),
        b"hmtx": hmtx([(500, 0), (250, 0), (1000, 0)]),
        # Version 0.5: no glyf, and only the number of glyphs.
        b"maxp": be(("L", 0x00005000), ("H", 3)),
        b"name": name("Kerf Test Bitmap", "KerfTestBitmap-Regular"),
        b"post": post(),
    })


if __name__ == "__main__":
    with open("KerfTestColr.ttf", "wb") as output:
        output.write(colr_font())
    with open("KerfTestBitmap.ttf", "wb") as output:
        output.write(bitmap_font())
