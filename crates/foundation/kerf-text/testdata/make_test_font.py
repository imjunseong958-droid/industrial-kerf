#!/usr/bin/env python3
"""Writes KerfTestVariable.ttf, the small variable font the tests set axes on.

No bundled font has variations, and Linux CI has no system font that does. This one is Kerf's own
work under the repository's license: a space and a capital I, whose stem is a plain rectangle, so
what a test finds in its ink can be checked against a number.

    I at wght 400 and wdth 100: stem 100 wide, 700 high, advance 300, in 1000 units to the em.
    wght 900 adds 200 to the stem and to the advance. wght 100 takes 60 from each.
    wdth 200 adds 100 to each.

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


def name():
    strings = {
        1: "Kerf Test Variable",
        2: "Regular",
        4: "Kerf Test Variable",
        6: "KerfTestVariable-Regular",
        256: "Weight",
        257: "Width",
    }
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


def fvar():
    table = be(("H", 1), ("H", 0), ("H", 16), ("H", 2), ("H", 2), ("H", 20), ("H", 0), ("H", 12))
    for tag, low, default, high, name_id in ((b"wght", 100, 400, 900, 256), (b"wdth", 50, 100, 200, 257)):
        table += tag + be(("l", fixed(low)), ("l", fixed(default)), ("l", fixed(high)), ("H", 0), ("H", name_id))
    return table


def gvar():
    def deltas(move):
        # The four corners and four phantom points; the second phantom point is the advance. The
        # right side of the stem moves, and the advance with it.
        across = [0, 0, move, move, 0, move, 0, 0]
        # Its own point numbers, of which a count of 0 means all of them.
        packed = bytes([0, 0x40 | (len(across) - 1)])
        for value in across:
            packed += struct.pack(">h", value)
        return packed + bytes([0x80 | (len(across) - 1)])

    tuples = [((1.0, 0.0), 200), ((-1.0, 0.0), -60), ((0.0, 1.0), 100)]
    headers = b""
    data = b""
    for (wght, wdth), move in tuples:
        serialized = deltas(move)
        headers += be(("H", len(serialized)), ("H", 0xA000), ("h", f2dot14(wght)), ("h", f2dot14(wdth)))
        data += serialized
    variation = be(("H", len(tuples)), ("H", 4 + len(headers))) + headers + data
    if len(variation) % 2:
        variation += b"\0"
    offsets = be(("L", 0), ("L", 0), ("L", 0), ("L", len(variation)))
    array_at = 20 + len(offsets)
    return (
        be(("H", 1), ("H", 0), ("H", 2), ("H", 0), ("L", array_at), ("H", 3), ("H", 1), ("L", array_at))
        + offsets
        + variation
    )


def font():
    outlines = glyf()
    tables = {
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
        b"name": name(),
        b"post": post(),
    }
    count = len(tables)
    power = 1
    while power * 2 <= count:
        power *= 2
    out = be(("L", 0x00010000), ("H", count), ("H", power * 16), ("H", len(bin(power)) - 3), ("H", count * 16 - power * 16))
    offset = 12 + 16 * count
    directory = b""
    body = b""
    for tag, table in sorted(tables.items()):
        padded = table + b"\0" * (-len(table) % 4)
        total = sum(struct.unpack(">%dL" % (len(padded) // 4), padded)) & 0xFFFFFFFF
        directory += tag + struct.pack(">LLL", total, offset + len(body), len(table))
        body += padded
    return out + directory + body


if __name__ == "__main__":
    with open("KerfTestVariable.ttf", "wb") as output:
        output.write(font())
