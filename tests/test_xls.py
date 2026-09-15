"""The legacy Excel reader: the records CIRO's report uses, and the container holding them."""
from __future__ import annotations

import struct
import unittest

import xls


def record(rid, body):
    return struct.pack("<HH", rid, len(body)) + body


def label(row, col, text, wide=False):
    body = struct.pack("<HHH", row, col, 0) + struct.pack("<HB", len(text), 1 if wide else 0)
    return record(xls.LABEL, body + (text.encode("utf-16le") if wide else text.encode("latin-1")))


def number(row, col, value):
    return record(xls.NUMBER, struct.pack("<HHH", row, col, 0) + struct.pack("<d", value))


def packed(value, cents=False):
    """An integer in the packed form, optionally in hundredths."""
    return ((value << 2) | 2 | (1 if cents else 0)) & 0xFFFFFFFF


def rk(row, col, raw):
    return record(xls.RK, struct.pack("<HHH", row, col, 0) + struct.pack("<I", raw))


def mulrk(row, first, raws):
    body = struct.pack("<HH", row, first)
    for raw in raws:
        body += struct.pack("<H", 0) + struct.pack("<I", raw)
    return record(xls.MULRK, body + struct.pack("<H", first + len(raws) - 1))


def sst(strings):
    body = struct.pack("<ii", len(strings), len(strings))
    for s in strings:
        body += struct.pack("<HB", len(s), 0) + s.encode("latin-1")
    return record(xls.SST, body)


def labelsst(row, col, index):
    return record(xls.LABELSST, struct.pack("<HHHi", row, col, 0, index))


def container(stream, name="Workbook"):
    """The smallest OLE compound file that holds one stream, laid out as the readers
    expect: a table sector, a directory sector, then the stream's own sectors."""
    sectors = (len(stream) + 511) // 512
    data = stream + b"\x00" * (sectors * 512 - len(stream))
    header = bytearray(512)
    header[0:8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"
    struct.pack_into("<H", header, 28, 0xFFFE)
    struct.pack_into("<H", header, 30, 9)          # 512-byte sectors
    struct.pack_into("<H", header, 32, 6)          # 64-byte short sectors
    struct.pack_into("<i", header, 44, 1)          # one table sector
    struct.pack_into("<i", header, 48, 1)          # the directory is sector 1
    struct.pack_into("<i", header, 56, 4096)       # anything smaller lives in the short stream
    struct.pack_into("<i", header, 60, -2)         # no short stream
    struct.pack_into("<i", header, 68, -2)         # no table overflow
    struct.pack_into("<i", header, 72, 0)
    for i in range(109):
        struct.pack_into("<i", header, 76 + i * 4, 0 if i == 0 else -1)
    table = bytearray(b"\xff" * 512)
    struct.pack_into("<i", table, 0, -3)           # sector 0 holds the table itself
    struct.pack_into("<i", table, 4, -2)           # the directory ends at once
    for i in range(sectors):
        struct.pack_into("<i", table, 8 + i * 4, -2 if i == sectors - 1 else 3 + i)
    directory = bytearray(512)
    def entry(at, entry_name, kind, start, size):
        raw = entry_name.encode("utf-16le")
        directory[at:at + len(raw)] = raw
        struct.pack_into("<H", directory, at + 64, len(raw) + 2)
        directory[at + 66] = kind
        struct.pack_into("<i", directory, at + 116, start)
        struct.pack_into("<I", directory, at + 120, size)
    entry(0, "Root Entry", 5, -2, 0)
    entry(128, name, 2, 2, len(stream))
    return bytes(header) + bytes(table) + bytes(directory) + data


class RecordTest(unittest.TestCase):
    def test_text_numbers_and_packed_runs_all_land_in_their_cells(self):
        stream = (label(0, 0, "Security Symbol") + label(0, 1, "Exchange Code") +
                  label(1, 0, "QNC") + label(1, 1, "TSXV") +
                  mulrk(1, 2, [packed(2667164), packed(64077)]) +
                  number(2, 0, 1.5) + rk(2, 1, packed(250, cents=True)) + rk(2, 2, struct.unpack("<I", struct.pack("<d", 2.0)[4:])[0] & 0xFFFFFFFC))
        cells = xls.cells(stream)
        self.assertEqual(cells[(0, 0)], "Security Symbol")
        self.assertEqual(cells[(1, 0)], "QNC")
        self.assertEqual(cells[(1, 2)], 2667164.0)
        self.assertEqual(cells[(1, 3)], 64077.0)
        self.assertEqual(cells[(2, 0)], 1.5)
        self.assertEqual(cells[(2, 1)], 2.5)          # the packed form in hundredths
        self.assertEqual(cells[(2, 2)], 2.0)          # the packed form as half a double

    def test_a_negative_packed_number_keeps_its_sign(self):
        self.assertEqual(xls.cells(rk(0, 0, packed(-110130)))[(0, 0)], -110130.0)

    def test_wide_text_reads_as_written(self):
        self.assertEqual(xls.cells(label(0, 0, "1911 GOLD", wide=True))[(0, 0)], "1911 GOLD")

    def test_shared_strings_are_read_through_the_cells_that_point_at_them(self):
        stream = sst(["ZYUS LIFE SCIENCES", "ZYUS"]) + labelsst(0, 0, 0) + labelsst(0, 1, 1) + labelsst(0, 2, 9)
        cells = xls.cells(stream)
        self.assertEqual(cells[(0, 0)], "ZYUS LIFE SCIENCES")
        self.assertEqual(cells[(0, 1)], "ZYUS")
        self.assertEqual(cells[(0, 2)], "")            # a cell pointing past the list is empty, not a crash

    def test_records_it_does_not_read_are_skipped_rather_than_breaking_the_row(self):
        stream = record(0x0208, b"\x00" * 16) + label(0, 0, "ONE") + record(0x00E0, b"\x01" * 20) + rk(0, 1, packed(395141))
        cells = xls.cells(stream)
        self.assertEqual(cells[(0, 0)], "ONE")
        self.assertEqual(cells[(0, 1)], 395141.0)


class ContainerTest(unittest.TestCase):
    def test_the_workbook_stream_is_found_and_read_as_a_table(self):
        stream = (label(0, 0, "Security Issue Name") + label(0, 1, "Security Symbol") + label(0, 2, "Exchange Code") +
                  label(1, 0, "HIGH TIDE INC.") + label(1, 1, "HITI") + label(1, 2, "TSXV") +
                  mulrk(1, 3, [packed(124186), packed(-10870)]))
        rows = xls.table(container(stream + b"\x00" * 5000))
        self.assertEqual(rows[0], ["Security Issue Name", "Security Symbol", "Exchange Code", "", ""])
        self.assertEqual(rows[1], ["HIGH TIDE INC.", "HITI", "TSXV", 124186.0, -10870.0])

    def test_something_that_is_not_a_container_says_so(self):
        with self.assertRaises(ValueError):
            xls.table(b"Security,Symbol\nHITI,TSXV\n")

    def test_a_container_without_a_workbook_says_so(self):
        with self.assertRaises(ValueError):
            xls.table(container(b"\x00" * 5000, name="Nothing"))


if __name__ == "__main__":
    unittest.main()
