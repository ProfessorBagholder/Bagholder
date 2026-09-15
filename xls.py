"""Just enough of the legacy Excel container to read one published table.

CIRO posts its short position report as a .xls of the 1997 kind: an OLE compound file
holding a stream of BIFF records. Only what such a report uses is implemented — text
cells, numbers, and the packed number runs — and every other record is skipped, so this
reads a table someone published and is in no sense a spreadsheet engine. It exists so the
report can be read with the standard library alone, rather than for a dependency whose
only job would be this one file.
"""
from __future__ import annotations

import struct

LABEL, NUMBER, RK, MULRK, SST, CONTINUE, LABELSST = 0x0204, 0x0203, 0x027E, 0x00BD, 0x00FC, 0x003C, 0x00FD


def streams(raw):
    """The named streams of an OLE compound file, as {name: bytes}."""
    if raw[:8] != b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1":
        raise ValueError("not an OLE compound file")
    ssz = 1 << struct.unpack_from("<H", raw, 30)[0]
    msz = 1 << struct.unpack_from("<H", raw, 32)[0]
    nfat, dirstart = struct.unpack_from("<ii", raw, 44)
    minicut, ministart = struct.unpack_from("<ii", raw, 56)
    difstart, ndif = struct.unpack_from("<ii", raw, 68)

    def sector(i):
        return raw[512 + i * ssz: 512 + (i + 1) * ssz]

    # the sectors holding the allocation table, listed in the header and then chained
    difat = list(struct.unpack_from("<109i", raw, 76))
    node, left = difstart, ndif
    while left > 0 and node >= 0:
        blk = sector(node)
        difat.extend(struct.unpack_from("<%di" % (ssz // 4 - 1), blk, 0))
        node = struct.unpack_from("<i", blk, ssz - 4)[0]
        left -= 1
    fat = []
    for f in difat[:nfat]:
        if f >= 0:
            fat.extend(struct.unpack_from("<%di" % (ssz // 4), sector(f), 0))

    def chain(start):
        out, i, seen = [], start, set()
        while i >= 0 and i < len(fat) and i not in seen:
            seen.add(i)
            out.append(i)
            i = fat[i]
        return out

    entries, dirdata = [], b"".join(sector(i) for i in chain(dirstart))
    for off in range(0, len(dirdata) - 127, 128):
        e = dirdata[off:off + 128]
        nlen = struct.unpack_from("<H", e, 64)[0]
        name = e[:max(0, nlen - 2)].decode("utf-16le", "replace")
        entries.append((name, e[66], struct.unpack_from("<i", e, 116)[0], struct.unpack_from("<I", e, 120)[0]))
    # a short stream lives inside the root entry's own stream, cut into smaller pieces
    root = next((e for e in entries if e[1] == 5), None)
    mini = b"".join(sector(i) for i in chain(root[2])) if root and root[2] >= 0 else b""
    minifat = []
    if ministart >= 0:
        blk = b"".join(sector(i) for i in chain(ministart))
        minifat = list(struct.unpack_from("<%di" % (len(blk) // 4), blk, 0))
    out = {}
    for name, kind, start, size in entries:
        if kind != 2:
            continue
        if size < minicut and minifat:
            buf, i, seen = b"", start, set()
            while i >= 0 and i < len(minifat) and i not in seen:
                seen.add(i)
                buf += mini[i * msz:(i + 1) * msz]
                i = minifat[i]
        else:
            buf = b"".join(sector(i) for i in chain(start))
        out[name] = buf[:size]
    return out


def records(buf):
    """The stream's records, as (id, body)."""
    i = 0
    while i + 4 <= len(buf):
        rid, ln = struct.unpack_from("<HH", buf, i)
        yield rid, buf[i + 4:i + 4 + ln]
        i += 4 + ln


def _number(v):
    """A packed number: the low two bits are flags, the rest is either a 30-bit signed
    integer or the top half of a double, and bit 0 means the value is in hundredths."""
    v &= 0xFFFFFFFF
    if v & 2:
        n = v >> 2
        x = float(n - (1 << 30) if n & (1 << 29) else n)
    else:
        x = struct.unpack("<d", struct.pack("<II", 0, v & 0xFFFFFFFC))[0]
    return x / 100 if v & 1 else x


def _text(data, off):
    """A string as the format writes one: a length, a flag saying how wide its
    characters are, then the characters."""
    n = struct.unpack_from("<H", data, off)[0]
    wide = data[off + 2] & 1
    body = data[off + 3: off + 3 + n * (2 if wide else 1)]
    return body.decode("utf-16le" if wide else "latin-1", "replace")


def _shared(chunks):
    """The workbook's shared strings, which cells then refer to by number."""
    buf = b"".join(chunks)
    count = struct.unpack_from("<i", buf, 4)[0]
    pos, out = 8, []
    for _ in range(count):
        if pos + 3 > len(buf):
            break
        n = struct.unpack_from("<H", buf, pos)[0]
        flags = buf[pos + 2]
        pos += 3
        rich = struct.unpack_from("<H", buf, pos)[0] if flags & 8 else 0
        pos += 2 if flags & 8 else 0
        far = struct.unpack_from("<i", buf, pos)[0] if flags & 4 else 0
        pos += 4 if flags & 4 else 0
        wide = flags & 1
        out.append(buf[pos: pos + n * (2 if wide else 1)].decode("utf-16le" if wide else "latin-1", "replace"))
        pos += n * (2 if wide else 1) + rich * 4 + far
    return out


def cells(buf):
    """{(row, column): value} for the records this reads, ignoring the rest."""
    out, shared, collecting = {}, [], None
    for rid, data in records(buf):
        if rid == SST:
            collecting = [data]
            continue
        if rid == CONTINUE and collecting is not None:
            collecting.append(data)
            continue
        if collecting is not None:
            shared = _shared(collecting)
            collecting = None
        if rid == LABEL:
            r, c = struct.unpack_from("<HH", data, 0)
            out[(r, c)] = _text(data, 6)
        elif rid == LABELSST:
            r, c, _, idx = struct.unpack_from("<HHHi", data, 0)
            out[(r, c)] = shared[idx] if 0 <= idx < len(shared) else ""
        elif rid == NUMBER:
            r, c = struct.unpack_from("<HH", data, 0)
            out[(r, c)] = struct.unpack_from("<d", data, 6)[0]
        elif rid == RK:
            r, c = struct.unpack_from("<HH", data, 0)
            out[(r, c)] = _number(struct.unpack_from("<I", data, 6)[0])
        elif rid == MULRK:
            r, first = struct.unpack_from("<HH", data, 0)
            for k in range((len(data) - 6) // 6):
                out[(r, first + k)] = _number(struct.unpack_from("<I", data, 6 + k * 6)[0])
    if collecting is not None:
        shared = _shared(collecting)
    return out


def table(raw):
    """The workbook's first stream as rows of cells, blanks filled in."""
    found = streams(raw)
    buf = found.get("Workbook") or found.get("Book")
    if not buf:
        raise ValueError("no workbook stream")
    grid = cells(buf)
    if not grid:
        return []
    rows, cols = max(r for r, _ in grid), max(c for _, c in grid)
    return [[grid.get((r, c), "") for c in range(cols + 1)] for r in range(rows + 1)]
