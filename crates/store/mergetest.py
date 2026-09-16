"""The sync merge, both ways.

A book is built in the order it really arrives: a first Wealthsimple pull, a
CSV import of rows the broker has not sent yet, a second pull that carries the
same fills (so the imported rows should be linked, not duplicated), a pull that
repeats rows already stored, one that revises a dividend from its record-date
placeholder to the paid amount, one that supplies a security id the first pull
lacked, and a CSV import with rows that duplicate each other.

The counts each implementation reports and the whole activities table are
compared after every step.

    cargo build -p bagholder-store && python3 crates/store/mergetest.py
"""
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
BIN = os.path.join(ROOT, "target", "debug", "storetool")


def ws(cid, day, symbol, qty, px, cash, **extra):
    row = {
        "canonicalId": cid, "id": cid, "transactionDate": day,
        "occurredAt": day + "T15:00:00+00:00", "accountId": "acct-1", "accountType": "Trading",
        "activityType": "Trade", "activitySubType": "BUY" if qty > 0 else "SELL",
        "rawType": "DIY_BUY" if qty > 0 else "DIY_SELL", "symbol": symbol, "currency": "CAD",
        "quantity": qty, "unitPrice": px, "commission": 0, "netCashAmount": cash,
        "category": "trade", "name": symbol, "source": "wealthsimple",
    }
    row.update(extra)
    return row


def csv_row(day, symbol, qty, px, cash, account="acct-1"):
    return {
        "transactionDate": day, "occurredAt": day, "accountId": account, "accountType": "Trading",
        "activityType": "Trade", "activitySubType": "BUY" if qty > 0 else "SELL",
        "symbol": symbol, "currency": "CAD", "quantity": qty, "unitPrice": px,
        "commission": 0, "netCashAmount": cash, "category": "trade", "source": "csv",
    }


# each step is (label, wealthsimple rows, local rows)
STEPS = [
    ("first pull", [
        ws("ws-1", "2026-01-05", "AAA", 100, 10.0, -1000.0),
        ws("ws-2", "2026-01-20", "AAA", -100, 12.0, 1200.0),
        ws("", "2026-01-21", "AAA", 1, 1.0, -1.0),            # no canonical id: skipped
        ws("manual-9", "2026-01-22", "AAA", 1, 1.0, -1.0),    # homemade id: skipped
    ], []),
    ("csv ahead of the broker", [], [
        csv_row("2026-02-02", "BBB", 50, 12.0, -600.0),
        csv_row("2026-02-10", "BBB", -50, 11.0, 550.0),
        csv_row("2026-02-02", "BBB", 50, 12.0, -600.0),       # duplicate of the first
    ]),
    ("pull carrying the same fills", [
        ws("ws-3", "2026-02-02", "BBB", 50, 12.0, -600.0),    # links to the imported row
        ws("ws-4", "2026-02-10", "BBB", -50, 11.0, 550.0),
    ], []),
    ("pull repeating what is stored", [
        ws("ws-1", "2026-01-05", "AAA", 100, 10.0, -1000.0),
        ws("ws-2", "2026-01-20", "AAA", -100, 12.0, 1200.0),
    ], []),
    ("dividend placeholder", [
        ws("ws-5", "2026-03-01", "DDD", 0, 0.0, 0.0, activityType="DIVIDEND", rawType="DIVIDEND",
           activitySubType="other", category="dividend"),
    ], []),
    ("the same dividend, paid", [
        ws("ws-5", "2026-03-15", "DDD", 300, 0.05, 15.0, activityType="DIVIDEND", rawType="DIVIDEND",
           activitySubType="other", category="dividend"),
    ], []),
    ("a security id the first pull lacked", [
        ws("ws-1", "2026-01-05", "AAA", 100, 10.0, -1000.0, securityId="sec-aaa"),
    ], []),
    ("two stored rows a csv row could be", [], [
        csv_row("2026-04-01", "CCC", 10, 5.0, -50.0),
        csv_row("2026-04-01", "CCC", 10, 5.0, -50.0),
    ]),
    ("a pull matching both of them", [
        ws("ws-6", "2026-04-01", "CCC", 10, 5.0, -50.0),
    ], []),
]


def table(path):
    c = sqlite3.connect(path)
    cols = [r[1] for r in c.execute("PRAGMA table_info(activities)")]
    # the generated id is not comparable, so it is dropped; canonical_id is kept
    keep = [x for x in cols if x != "id"]
    rows = sorted(c.execute(f"SELECT {', '.join(keep)} FROM activities"), key=repr)
    c.close()
    return rows


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


def main():
    work = tempfile.mkdtemp(prefix="mergetest-")
    pyhome = os.path.join(work, "py")
    rshome = os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)

    import store
    store.set_home(pyhome)
    store.ensure()
    store.close_all()
    shutil.copy(os.path.join(pyhome, "bagholder.db"), os.path.join(rshome, "bagholder.db"))
    pydb = os.path.join(pyhome, "bagholder.db")
    rsdb = os.path.join(rshome, "bagholder.db")

    counter = {"n": 0}

    def gen():
        i = counter["n"]
        counter["n"] += 1
        return f"gen-{i}"

    bad = []
    for label, wsrows, localrows in STEPS:
        id_start = counter["n"]
        store.set_home(pyhome)
        real = store._new_id
        store._new_id = gen
        try:
            applied = store.apply_wealthsimple_mapped(wsrows)
            merged = store.merge_local_rows(localrows)
        finally:
            store._new_id = real
        store.close_all()

        r = subprocess.run([BIN, "merge", rsdb],
                           input=json.dumps({"ws": wsrows, "local": localrows, "idStart": id_start}),
                           capture_output=True, text=True)
        if r.returncode != 0:
            print(r.stderr[-2000:])
            return 1
        got = json.loads(r.stdout)

        if norm(applied) != norm(got["applied"]):
            bad.append(f"{label}: applied py={applied} rs={got['applied']}")
        want_merged = {k: merged[k] for k in ("ok", "added", "duplicates")}
        got_merged = {k: got["merged"][k] for k in ("ok", "added", "duplicates")}
        if norm(want_merged) != norm(got_merged):
            bad.append(f"{label}: merged py={want_merged} rs={got_merged}")

        a, b = table(pydb), table(rsdb)
        if a != b:
            bad.append(f"{label}: table differs ({len(a)} python rows, {len(b)} rust)")
            for x, y in zip(a, b):
                if x != y:
                    bad.append(f"    py={x}")
                    bad.append(f"    rs={y}")
                    break

    for line in bad[:30]:
        print("  " + line)
    print(f"{len(STEPS)} steps, {len(table(pydb))} rows, {len(bad)} differences")
    shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
