#!/usr/bin/env python3
"""Writes KerfTestVariable.ttf and KerfTestCff2.otf, the small variable fonts the tests set axes on.

No bundled font has variations, and Linux CI has no system font that does. These are Kerf's own
work under the repository's license: a space and a capital I, whose stem is a plain rectangle, so
what a test finds in its ink can be checked against a number.

    I at its defaults: stem 100 wide, 700 high, advance 300, in 1000 units to the em.
    wght 900 adds 200 to the stem and to the advance. wght 100 takes 60 from each.
    wdth 200 adds 100 to each.
    opsz 144 takes 50 from each, and opsz 8 adds 50.
    slnt -20 moves the top of the stem 255 to the right, and leaves the advance.
    ital 1 moves the top of the stem 120 to the right, and takes 20 from the advance.

KerfTestVariable.ttf draws it in TrueType outlines with `gvar`, and has all five axes.
KerfTestCff2.otf draws it in CFF2 with `HVAR`, and has only wght.

Run it from this folder with `python3 make_test_font.py`.
"""
import struct


def be(*fields):
    """Packs (format, value) pairs big-endian."""
    return b"".join(struct.pack(">" + kind, value) for kind, value in fields)


def f2dot14(value):
    return int(round(value * 16384))


def fixed(value):
    return int(round(value * 65536))


def head():
    return be(
        ("L", 0x00010000), ("L", 0x00010000), ("L", 0), ("L", 0x5F0F3CF5), ("H", 0x000B), ("H", 1000),
        ("q", 0), ("q", 0), ("h", 0), ("h", 0), ("h", 300), ("h", 700), ("H", 0), ("H", 8), ("h", 2),
        ("h", 1), ("h", 0),
    )


def hhea():
    return be(
        ("L", 0x00010000), ("h", 800), ("h", -200), ("h", 0), ("H", 500), ("h", 0), ("h", 0), ("h", 300),
        ("h", 1), ("h", 0), ("h", 0), ("h", 0), ("h", 0), ("h", 0), ("h", 0), ("h", 0), ("H", 3),
    )


def maxp():
    return be(("L", 0x00010000), ("H", 3), ("H", 4), ("H", 1)) + b"\0" * 2 * 11


def hmtx():
    return be(("H", 500), ("h", 0), ("H", 250), ("h", 0), ("H", 300), ("h", 100))


def glyf():
    """The I: a rectangle from (100, 0) to (200, 700), clockwise."""
    glyph = be(("h", 1), ("h", 100), ("h", 0), ("h", 200), ("h", 700), ("H", 3), ("H", 0))
    glyph += bytes([1, 1, 1, 1])
    for delta in (100, 0, 100, 0, 0, 700, 0, -700):
        glyph += struct.pack(">h", delta)
    return glyph


def loca(length):
    return be(("L", 0), ("L", 0), ("L", 0), ("L", length))


def cmap():
    groups = [(0x20, 0x20, 1), (0x49, 0x49, 2)]
    subtable = be(("H", 12), ("H", 0), ("L", 16 + 12 * len(groups)), ("L", 0), ("L", len(groups)))
    for group in groups:
        subtable += struct.pack(">LLL", *group)
    return be(("H", 0), ("H", 1), ("H", 3), ("H", 10), ("L", 12)) + subtable


def name(family="Kerf Test Variable", postscript="KerfTestVariable-Regular", axes=()):
    strings = {1: family, 2: "Regular", 4: family, 6: postscript}
    for name_id, text in axes:
        strings[name_id] = text
    records = b""
    storage = b""
    for id_, text in strings.items():
        encoded = text.encode("utf-16-be")
        records += struct.pack(">HHHHHH", 3, 1, 0x0409, id_, len(encoded), len(storage))
        storage += encoded
    return be(("H", 0), ("H", len(strings)), ("H", 6 + len(records))) + records + storage


def os2():
    table = bytearray(96)
    struct.pack_into(">H", table, 0, 4)
    struct.pack_into(">H", table, 4, 400)
    struct.pack_into(">H", table, 6, 5)
    struct.pack_into(">H", table, 62, 0x40)
    struct.pack_into(">h", table, 68, 800)
    struct.pack_into(">h", table, 70, -200)
    return bytes(table)


def post():
    return be(("L", 0x00030000)) + b"\0" * 28


# The axes of KerfTestVariable.ttf: tag, least, default, most, name id, name.
AXES = (
    (b"wght", 100, 400, 900, 256, "Weight"),
    (b"wdth", 50, 100, 200, 257, "Width"),
    (b"opsz", 8, 14, 144, 258, "Optical Size"),
    (b"slnt", -20, 0, 0, 259, "Slant"),
    (b"ital", 0, 0, 1, 260, "Italic"),
)


def fvar(axes=AXES):
    table = be(("H", 1), ("H", 0), ("H", 16), ("H", 2), ("H", len(axes)), ("H", 20), ("H", 0), ("H", 4 + 4 * len(axes)))
    for tag, low, default, high, name_id, _ in axes:
        table += tag + be(("l", fixed(low)), ("l", fixed(default)), ("l", fixed(high)), ("H", 0), ("H", name_id))
    return table


def gvar():
    def deltas(top=0, stem=0, advance=0):
        # The four corners, from the bottom left round to the bottom right, and four phantom
        # points, the second of which is the advance.
        across = [0, top, top + stem, stem, 0, advance, 0, 0]
        # Its own point numbers, of which a count of 0 means all of them.
        packed = bytes([0, 0x40 | (len(across) - 1)])
        for value in across:
            packed += struct.pack(">h", value)
        return packed + bytes([0x80 | (len(across) - 1)])

    # What each axis does at its far end, by its place in AXES. The right side of the stem moves
    # and the advance with it, or the top of the stem leans.
    tuples = [
        ((1.0, 0.0, 0.0, 0.0, 0.0), deltas(stem=200, advance=200)),
        ((-1.0, 0.0, 0.0, 0.0, 0.0), deltas(stem=-60, advance=-60)),
        ((0.0, 1.0, 0.0, 0.0, 0.0), deltas(stem=100, advance=100)),
        ((0.0, 0.0, 1.0, 0.0, 0.0), deltas(stem=-50, advance=-50)),
        ((0.0, 0.0, -1.0, 0.0, 0.0), deltas(stem=50, advance=50)),
        ((0.0, 0.0, 0.0, -1.0, 0.0), deltas(top=255)),
        ((0.0, 0.0, 0.0, 0.0, 1.0), deltas(top=120, advance=-20)),
    ]
    headers = b""
    data = b""
    for peak, serialized in tuples:
        headers += be(("H", len(serialized)), ("H", 0xA000))
        for coordinate in peak:
            headers += be(("h", f2dot14(coordinate)))
        data += serialized
    variation = be(("H", len(tuples)), ("H", 4 + len(headers))) + headers + data
    if len(variation) % 2:
        variation += b"\0"
    offsets = be(("L", 0), ("L", 0), ("L", 0), ("L", len(variation)))
    array_at = 20 + len(offsets)
    return (
        be(("H", 1), ("H", 0), ("H", len(AXES)), ("H", 0), ("L", array_at), ("H", 3), ("H", 1), ("L", array_at))
        + offsets
        + variation
    )


def number(value):
    """A number as a CFF2 charstring or DICT writes it."""
    if -107 <= value <= 107:
        return bytes([value + 139])
    return struct.pack(">Bh", 28, value)


def index(items):
    """A CFF2 INDEX, whose count has four bytes."""
    if not items:
        return struct.pack(">L", 0)
    out = struct.pack(">LB", len(items), 2)
    at = 1
    out += struct.pack(">H", at)
    for item in items:
        at += len(item)
        out += struct.pack(">H", at)
    return out + b"".join(items)


def regions():
    """Two regions of wght: towards its heavy end, and towards its light one."""
    list_ = be(("H", 1), ("H", 2))
    for start, peak, end in ((0.0, 1.0, 1.0), (-1.0, -1.0, 0.0)):
        list_ += be(("h", f2dot14(start)), ("h", f2dot14(peak)), ("h", f2dot14(end)))
    return list_


def variation_store(items):
    """An ItemVariationStore over regions(), with int16 deltas for those two for each of `items`."""
    data = be(("H", len(items)), ("H", 2), ("H", 2), ("H", 0), ("H", 1))
    for heavy, light in items:
        data += be(("h", heavy), ("h", light))
    region_list = regions()
    regions_at = 8 + 4
    return be(("H", 1), ("L", regions_at), ("H", 1), ("L", regions_at + len(region_list))) + region_list + data


def cff2():
    """The I: the same rectangle, drawn from its bottom left; `blend` moves its right side."""
    stem = number(100) + number(200) + number(-60) + number(1) + bytes([16])
    back = number(-100) + number(-200) + number(60) + number(1) + bytes([16])
    glyph = number(100) + number(0) + bytes([21]) + stem + bytes([6]) + number(700) + bytes([7]) + back + bytes([6])
    charstrings = index([b"", b"", glyph])
    store = variation_store([])
    store = struct.pack(">H", len(store)) + store
    private = number(0) + bytes([22])
    subrs = index([])

    def top(charstrings_at, fd_array_at, store_at):
        return (
            struct.pack(">Bl", 29, charstrings_at) + bytes([17])
            + struct.pack(">Bl", 29, fd_array_at) + bytes([12, 36])
            + struct.pack(">Bl", 29, store_at) + bytes([24])
        )

    # Everything's place depends on the size of what comes first, which does not depend on places.
    head_size = 5 + len(top(0, 0, 0))
    store_at = head_size + len(subrs)
    charstrings_at = store_at + len(store)
    fd_array_at = charstrings_at + len(charstrings)

    def font_dict(private_at):
        return struct.pack(">BlBl", 29, len(private), 29, private_at) + bytes([18])

    fd_array_size = len(index([font_dict(0)]))
    private_at = fd_array_at + fd_array_size
    fd_array = index([font_dict(private_at)])
    top_dict = top(charstrings_at, fd_array_at, store_at)
    return bytes([2, 0, 5]) + struct.pack(">H", len(top_dict)) + top_dict + subrs + store + charstrings + fd_array + private


def hvar():
    # The advance goes as the stem does; the implicit map takes each glyph to its own item.
    store = variation_store([(0, 0), (0, 0), (200, -60)])
    return be(("H", 1), ("H", 0), ("L", 20), ("L", 0), ("L", 0), ("L", 0)) + store


def sfnt(version, tables):
    count = len(tables)
    power = 1
    while power * 2 <= count:
        power *= 2
    out = be(("L", version), ("H", count), ("H", power * 16), ("H", len(bin(power)) - 3), ("H", count * 16 - power * 16))
    offset = 12 + 16 * count
    directory = b""
    body = b""
    for tag, table in sorted(tables.items()):
        padded = table + b"\0" * (-len(table) % 4)
        total = sum(struct.unpack(">%dL" % (len(padded) // 4), padded)) & 0xFFFFFFFF
        directory += tag + struct.pack(">LLL", total, offset + len(body), len(table))
        body += padded
    return out + directory + body


def font():
    outlines = glyf()
    return sfnt(0x00010000, {
        b"OS/2": os2(),
        b"cmap": cmap(),
        b"fvar": fvar(),
        b"glyf": outlines + b"\0" * (-len(outlines) % 4),
        b"gvar": gvar(),
        b"head": head(),
        b"hhea": hhea(),
        b"hmtx": hmtx(),
        b"loca": loca(len(outlines) + (-len(outlines) % 4)),
        b"maxp": maxp(),
        b"name": name(axes=[(axis[4], axis[5]) for axis in AXES]),
        b"post": post(),
    })


def cff2_font():
    axes = AXES[:1]
    return sfnt(0x4F54544F, {
        b"CFF2": cff2(),
        b"HVAR": hvar(),
        b"OS/2": os2(),
        b"cmap": cmap(),
        b"fvar": fvar(axes),
        b"head": head(),
        b"hhea": hhea(),
        b"hmtx": hmtx(),
        # Version 0.5: no glyf, and only the number of glyphs.
        b"maxp": be(("L", 0x00005000), ("H", 3)),
        b"name": name("Kerf Test CFF2", "KerfTestCff2-Regular", [(axes[0][4], axes[0][5])]),
        b"post": post(),
    })


if __name__ == "__main__":
    with open("KerfTestVariable.ttf", "wb") as output:
        output.write(font())
    with open("KerfTestCff2.otf", "wb") as output:
        output.write(cff2_font())
