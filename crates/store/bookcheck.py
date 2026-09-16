"""The whole pipeline against a real book.

The thirty-three shared cases are small by design. This takes a copy of an
actual database, reads it with both stores, and runs both models over it, then
compares the snapshot and every figure the page shows.

It never touches the live database: the file is copied first, and both sides
read their own copy. Point it at one with BAGHOLDER_DB, or let it take
~/.bagholder/bagholder.db.

    cargo build && python3 crates/store/bookcheck.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
sys.path.insert(0, os.path.join(ROOT, "tests"))
STORETOOL = os.path.join(ROOT, "target", "debug", "storetool")
CASETOOL = os.path.join(ROOT, "target", "debug", "casetool")
VIEWTOOL = os.path.join(ROOT, "target", "debug", "viewtool")

LIVE = os.environ.get("BAGHOLDER_DB") or os.path.expanduser("~/.bagholder/bagholder.db")


def diff(path, want, got, out, limit=25):
    if len(out) > limit:
        return
    if isinstance(want, dict) and isinstance(got, dict):
        for k in sorted(set(want) | set(got)):
            if k not in want:
                out.append(f"{path}.{k}: only rust")
            elif k not in got:
                out.append(f"{path}.{k}: only python")
            else:
                diff(f"{path}.{k}", want[k], got[k], out, limit)
    elif isinstance(want, list) and isinstance(got, list):
        if len(want) != len(got):
            out.append(f"{path}: {len(want)} rows in python, {len(got)} in rust")
            return
        for i, (w, g) in enumerate(zip(want, got)):
            diff(f"{path}[{i}]", w, g, out, limit)
    elif isinstance(want, bool) or isinstance(got, bool):
        if want is not got:
            out.append(f"{path}: py={want!r} rs={got!r}")
    elif isinstance(want, (int, float)) and isinstance(got, (int, float)):
        if abs(float(want) - float(got)) > 5e-7:
            out.append(f"{path}: py={want!r} rs={got!r}")
    elif want != got:
        out.append(f"{path}: py={want!r} rs={got!r}")


def main():
    if not os.path.exists(LIVE):
        print(f"no database at {LIVE}; set BAGHOLDER_DB")
        return 0

    work = tempfile.mkdtemp(prefix="bookcheck-")
    pyhome = os.path.join(work, "py")
    rshome = os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)
    # never the live file: each side gets its own copy
    shutil.copy(LIVE, os.path.join(pyhome, "bagholder.db"))
    shutil.copy(LIVE, os.path.join(rshome, "bagholder.db"))
    rsdb = os.path.join(rshome, "bagholder.db")

    import store
    import model
    import make_cases

    store.set_home(pyhome)
    snap = store.snapshot()
    market = store.market_data()
    journal = store.journal()
    today = model.today_local()
    store.close_all()

    bad = []

    got_snap = json.loads(subprocess.run([STORETOOL, "snapshot", rsdb],
                                         capture_output=True, text=True, check=True).stdout)
    diff("snapshot", snap, got_snap, bad)

    got_journal = json.loads(subprocess.run([STORETOOL, "journal", rsdb],
                                            capture_output=True, text=True, check=True).stdout)
    diff("journal", journal, got_journal, bad)

    got_market = json.loads(subprocess.run([STORETOOL, "market", rsdb],
                                           capture_output=True, text=True, check=True).stdout)
    diff("market", market, got_market, bad)

    case = {"snapshot": snap, "market": market, "today": today, "journal": journal, "filters": {}}
    want_view = make_cases.expect_from(snap, market, today, {}, journal)
    got_view = json.loads(subprocess.run([CASETOOL], input=json.dumps(case),
                                         capture_output=True, text=True, check=True).stdout)
    diff("view", want_view, got_view, bad)

    # the whole payload, not only the projection a case records
    base = model.build_base(snap, market, journal, today=today)
    want_full = model.build_view(base, None)
    got_full = json.loads(subprocess.run([VIEWTOOL], input=json.dumps(case),
                                         capture_output=True, text=True, check=True).stdout)
    for key in want_full:
        if key == "generated":
            continue
        diff("full." + key, want_full[key], got_full.get(key), bad)

    for line in bad[:25]:
        print("  " + line)
    mk = want_full["markets"]
    print(f"{len(snap['activities'])} activities, {len(market['quotes'])} quotes, "
          f"{len(want_view['trades'])} trades, {len(want_view['positions'])} positions, "
          f"{len(mk['news'])} news rows, {len(mk['tiles'])} tiles, {len(bad)} differences")
    shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
