"""The rest of the store, both ways: the securities table, the journal
writers, the saved tile row, which security ids are missing, whether a
backfill is owed, when the next pull is due, and the wipe.

The pull schedule is checked at the minute either side of the close, on a
weekend, and after a pull has already been marked -- and in the app's own time
zone, not UTC.

    cargo build -p bagholder-store && python3 crates/store/admintest.py
"""
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
BIN = os.path.join(ROOT, "target", "debug", "storetool")

NOW = "2026-09-16T12:00:00Z"

SECURITIES = [
    {"id": "sec-1", "symbol": "AAA", "name": "Triple A", "primaryExchange": "TSX",
     "primaryMic": "XTSX", "currency": "CAD"},
    {"id": "sec-2", "symbol": "ZZZ 21AUG26 10.00 CALL", "name": "", "primary_exchange": "",
     "primary_mic": "", "currency": "USD", "underlyingId": "sec-1"},
    {"id": "sec-1", "symbol": "AAA", "name": "Triple A Corp", "primaryExchange": "TSX",
     "primaryMic": "XTSX", "currency": "CAD", "fetchedAt": "2026-09-01T00:00:00Z"},
    {"id": "  ", "symbol": "dropped"},
    {"id": "sec-3", "symbol": "CCC", "underlying_id": "  "},
    "not a dict",
]

JOURNAL = {
    "rt:1": {"thesis": "Breakout.", "tags": ["earnings", "breakout", "earnings"], "grade": "a"},
    "rt:2": {"thesis": "", "tags": "income, dividends", "grade": "Z"},
    "rt:3": {"thesis": "", "tags": [], "grade": ""},
    " ": {"grade": "A"},
    "rt:4": "not a dict",
}

JOURNAL_ENTRIES = [
    ["rt:5", {"thesis": "Added later.", "grade": "B", "tags": ["new"]}],
    ["rt:1", {"thesis": "", "grade": "", "tags": []}],     # empties it: the key goes
    ["  ", {"grade": "A"}],
    ["rt:6", None],
]

TILES = [
    {"symbol": " spx ", "exchange": " index "},
    {"symbol": "GC", "exchange": "comex"},
    {"symbol": "", "exchange": "INDEX"},
    "not a dict",
]

MISSING = ["sec-1", "sec-9", "sec-9", "  ", "sec-2"]

# 2026-09-16 is a Wednesday
PULL_TIMES = [
    int(datetime(2026, 9, 16, 19, 0, tzinfo=timezone.utc).timestamp()),   # 13:00 local: before
    int(datetime(2026, 9, 16, 20, 1, tzinfo=timezone.utc).timestamp()),   # 14:01 local: due
    int(datetime(2026, 9, 19, 20, 1, tzinfo=timezone.utc).timestamp()),   # Saturday
    int(datetime(2026, 9, 20, 20, 1, tzinfo=timezone.utc).timestamp()),   # Sunday
    int(datetime(2026, 9, 18, 20, 1, tzinfo=timezone.utc).timestamp()),   # Friday: due
]


def norm(v):
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return round(float(v), 9) + 0.0
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [norm(x) for x in v]
    return v


def tables(path):
    c = sqlite3.connect(path)
    out = {}
    for t in ("securities", "meta", "activities", "accounts", "balances", "nav_history",
              "grouped_trades", "fx_rates", "benchmark_prices", "quotes"):
        cols = [r[1] for r in c.execute(f"PRAGMA table_info({t})")]
        order = ", ".join(cols)
        out[t] = list(c.execute(f"SELECT {order} FROM {t} ORDER BY {order}"))
    c.close()
    return out


def main():
    work = tempfile.mkdtemp(prefix="admintest-")
    pyhome = os.path.join(work, "py")
    rshome = os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)

    import store
    store._now_iso = lambda: NOW
    for home in (pyhome, rshome):
        store.set_home(home)
        store.ensure()
        store.close_all()

    store.set_home(pyhome)
    # a broker row with a symbol and no security id: the backfill is owed
    store.insert_activity({"id": "ws-1", "source": "wealthsimple", "canonicalId": "ws-1",
                           "transactionDate": "2026-05-04", "symbol": "AAA", "quantity": 1,
                           "unitPrice": 1, "netCashAmount": -1, "activitySubType": "BUY",
                           "category": "trade"})
    store.upsert_securities(SECURITIES)
    journal = store.save_journal(JOURNAL)
    entries = [store.save_journal_entry(k, e) for k, e in JOURNAL_ENTRIES]
    tiles = store.save_tiles(TILES)
    want = {
        "securities": store.list_securities(),
        "journal": journal,
        "journalEntries": entries,
        "tiles": tiles,
        "missing": store.missing_security_ids(MISSING),
        "needsBackfill": store.needs_security_id_backfill(),
        "exposures": store.exposures_map(),
        "pullDue": [store.activity_pull_due(datetime.fromtimestamp(t, timezone.utc)) for t in PULL_TIMES],
    }
    store.close_all()

    # the same starting row on the Rust side
    store.set_home(rshome)
    store.insert_activity({"id": "ws-1", "source": "wealthsimple", "canonicalId": "ws-1",
                           "transactionDate": "2026-05-04", "symbol": "AAA", "quantity": 1,
                           "unitPrice": 1, "netCashAmount": -1, "activitySubType": "BUY",
                           "category": "trade"})
    store.close_all()

    payload = {"now": NOW, "securities": SECURITIES, "journal": JOURNAL,
               "journalEntries": JOURNAL_ENTRIES, "tiles": TILES, "missing": MISSING,
               "pullTimes": PULL_TIMES}
    r = subprocess.run([BIN, "admin", os.path.join(rshome, "bagholder.db")],
                       input=json.dumps(payload), capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stderr[-2000:])
        return 1
    got = json.loads(r.stdout)

    bad = []
    for key in want:
        if norm(want[key]) != norm(got.get(key)):
            bad.append(f"{key}:\n    py={json.dumps(norm(want[key]))[:300]}\n    rs={json.dumps(norm(got.get(key)))[:300]}")

    a, b = tables(os.path.join(pyhome, "bagholder.db")), tables(os.path.join(rshome, "bagholder.db"))
    for t in a:
        if a[t] != b[t]:
            bad.append(f"table {t} before the wipe:\n    py={a[t]}\n    rs={b[t]}")

    # and the wipe itself
    store.set_home(pyhome)
    store.clear_synced_data(keep_journal=True, keep_market=True)
    store.close_all()
    subprocess.run([BIN, "wipe", os.path.join(rshome, "bagholder.db")],
                   input=json.dumps({"keepJournal": True, "keepMarket": True}),
                   capture_output=True, text=True, check=True)
    a, b = tables(os.path.join(pyhome, "bagholder.db")), tables(os.path.join(rshome, "bagholder.db"))
    for t in a:
        if a[t] != b[t]:
            bad.append(f"table {t} after the wipe:\n    py={a[t]}\n    rs={b[t]}")

    for line in bad[:20]:
        print("  " + line)
    print(f"{len(want)} readers, {len(a)} tables through a wipe, {len(bad)} differences")
    shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
