"""Differential test for xls.py against crates/market/src/xls.rs.

The reader exists for one file: CIRO's short position report. The harness
fetches the real reports -- several reporting dates, so a run of them is read
rather than one -- and compares every cell of the table the two readers build,
row by row and column by column. A truncated file and a file that is not an
OLE container at all are read too, so the two agree on refusing as well as on
reading.
"""
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")))
import market  # noqa: E402
import shorts  # noqa: E402
import xls  # noqa: E402

TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "target", "release", "xlstool")
WORK = os.environ.get("XLSTEST_DIR") or "/tmp/xlstest"


def fetch(url, path):
    if os.path.exists(path):
        return open(path, "rb").read()
    # the app's own fetch, with the certificate store it uses
    raw = market._fetch(url, market.default_ssl_context(), shorts.HEADERS, market.TIMEOUT_SEC)
    open(path, "wb").write(raw)
    return raw


def main():
    os.makedirs(WORK, exist_ok=True)
    from datetime import datetime, timezone
    files, bad = [], []
    for d in shorts.position_dates(datetime.now(timezone.utc).date(), back=6):
        url = shorts.CA_POSITION_URL % d.strftime("%Y%m%d")
        path = os.path.join(WORK, "%s.xls" % d.strftime("%Y%m%d"))
        try:
            raw = fetch(url, path)
        except Exception as e:
            print("  no report for %s (%s)" % (d, e))
            continue
        if raw[:8] == b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1":
            files.append(path)
    if not files:
        print("no CIRO report answered; nothing to compare")
        return 0

    # a file cut short, and one that is not an OLE container at all
    whole = open(files[0], "rb").read()
    cut = os.path.join(WORK, "cut.xls")
    open(cut, "wb").write(whole[:len(whole) // 3])
    junk = os.path.join(WORK, "junk.xls")
    open(junk, "wb").write(b"symbol,shares\nAAA,1\n")
    empty = os.path.join(WORK, "empty.xls")
    open(empty, "wb").write(b"")
    files += [cut, junk, empty, os.path.join(WORK, "absent.xls")]

    # a real report cut at many lengths and with bytes flipped in it: the two
    # readers must agree on what they refuse as closely as on what they read
    import random
    rnd = random.Random(20260916)
    for k in range(40):
        c = os.path.join(WORK, "cut%02d.xls" % k)
        open(c, "wb").write(whole[:rnd.randint(1, len(whole))])
        files.append(c)
    for k in range(40):
        b = bytearray(whole)
        for _ in range(rnd.randint(1, 8)):
            b[rnd.randrange(len(b))] = rnd.randrange(256)
        f = os.path.join(WORK, "flip%02d.xls" % k)
        open(f, "wb").write(bytes(b))
        files.append(f)

    p = subprocess.run([TOOL], input=json.dumps({"files": files}), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("xlstool failed: %s" % p.stderr)
    got = json.loads(p.stdout)

    n = 0
    for i, path in enumerate(files):
        try:
            found = xls.streams(open(path, "rb").read())
            buf = found.get("Workbook") or found.get("Book")
            if not buf:
                raise ValueError("no workbook stream")
            want = xls.cells(buf)
            ok = True
        except Exception:
            want, ok = None, False
        if ok != got[i]["ok"]:
            bad.append("%s: python %s, rust %s (%s)" % (os.path.basename(path), "read" if ok else "refused",
                                                        "read" if got[i]["ok"] else "refused", got[i].get("error", "")))
            continue
        if not ok:
            n += 1
            continue
        nrows = (max(r for r, _ in want) + 1) if want else 0
        cols = (max(c for _, c in want) + 1) if want else 0
        filled = [[r, c, v] for (r, c), v in sorted(want.items()) if v != ""]
        name = os.path.basename(path)
        if (nrows, cols) != (got[i]["rows"], got[i]["cols"]):
            bad.append("%s: %dx%d python, %dx%d rust" % (name, nrows, cols, got[i]["rows"], got[i]["cols"]))
        elif filled != got[i]["cells"]:
            for x, y in zip(filled, got[i]["cells"]):
                if x != y:
                    bad.append("%s cell %r: python %r rust %r" % (name, x[:2], x[2], y))
                    break
            else:
                bad.append("%s: %d filled cells python, %d rust" % (name, len(filled), len(got[i]["cells"])))
        n += max(1, len(filled))
        if not name.startswith(("cut", "flip")):
            print("  %-16s %5d rows x %3d columns" % (name, nrows, cols))

    print("%d comparisons, %d differing" % (n, len(bad)))
    for b in bad[:20]:
        print("  " + b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
