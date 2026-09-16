"""Both `ensure()` implementations against the same database.

A fresh schema is written, raw Wealthsimple option rows are inserted under the
broker's own labels, the file is copied, and each implementation is run on its
own copy. Every table is then compared row for row, so the relabelling and the
one-shot unit-price scaling cannot differ.

    cargo build -p bagholder-store && python3 crates/store/ensuretest.py
"""
import os
import random
import shutil
import sqlite3
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
BIN = os.path.join(ROOT, "target", "debug", "schematool")

RAW_TYPES = ["OPTIONS_BUY", "OPTIONS_SELL", "OPTIONS_MULTILEG", "OPTIONS_ASSIGNMENT",
             "OPTIONS_EXPIRY", "OPTIONS_SHORT_EXPIRY", "DIY_BUY", "DIY_SELL", "DIVIDEND"]
SUBS = ["LIMIT_ORDER", "MARKET_ORDER", "other", "BUYTOOPEN", "SELLTOOPEN", "BUYTOCLOSE",
        "SELLTOCLOSE", "BTO", "STO", "BTC", "STC", "COVER", "BUY", "SELL", "FILLED", "ASSIGNED"]
SYMBOLS = ["ZZZ 21AUG26 10.00 CALL", "QQQ 20MAR26 5.00 PUT", "AAA", "LUNR 29AUG25 11.50 C",
           "BBB 15MAY26 3.00 P", "", "ETH"]


def seed(path, n=400):
    """A schema written by the Python store, then rows as the broker posts them."""
    import store
    home = os.path.dirname(path)
    store.set_home(home)
    store.ensure()
    store.close_all()
    rng = random.Random(11)
    c = sqlite3.connect(path)
    for i in range(n):
        qty = rng.choice([-5, -2, -1, 0, 1, 2, 3, 16, 100])
        px = rng.choice([0.0, 0.03, 0.2, 1.35, 2.0, 18.3, 120.0, 250.0])
        # a mix of contract cash and per-share cash, which is what the scaling reads
        cash = rng.choice([0.0, abs(qty) * px, abs(qty) * px * 100, -abs(qty) * px * 100, 137.5])
        c.execute(
            "INSERT INTO activities (id, transaction_date, occurred_at, account_id, account_type, "
            "activity_type, activity_sub_type, raw_type, symbol, currency, quantity, unit_price, "
            "commission, net_cash_amount, category, source) "
            "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (f"a{i}", "2026-%02d-%02d" % (rng.randint(1, 12), rng.randint(1, 28)),
             "2026-01-01T15:00:00+00:00", "acct-1", "Trading",
             rng.choice(["Trade", "LIMIT_ORDER", "other"]), rng.choice(SUBS), rng.choice(RAW_TYPES),
             rng.choice(SYMBOLS), rng.choice(["CAD", "USD"]), qty, px, 0.0, cash,
             rng.choice(["trade", "other", "option_event", "dividend"]), "wealthsimple"))
    # the stamps that would let the migrations skip must not be present
    c.execute("DELETE FROM meta WHERE key IN ('option_relabel_rows_v1', 'option_unit_price_scale_v1')")
    c.commit()
    c.close()


def tables(path):
    c = sqlite3.connect(path)
    names = [r[0] for r in c.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    out = {}
    for t in names:
        cols = [r[1] for r in c.execute(f"PRAGMA table_info({t})")]
        rows = list(c.execute(f"SELECT {', '.join(cols)} FROM {t} ORDER BY {', '.join(cols)}"))
        out[t] = (cols, rows)
    c.close()
    return out


def main():
    work = tempfile.mkdtemp(prefix="ensuretest-")
    pyhome = os.path.join(work, "py")
    rshome = os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)
    seeded = os.path.join(pyhome, "bagholder.db")
    seed(seeded)
    shutil.copy(seeded, os.path.join(rshome, "bagholder.db"))

    import store
    store.set_home(pyhome)
    store.ensure()
    store.close_all()

    subprocess.run([BIN, os.path.join(rshome, "bagholder.db")], check=True)

    a = tables(seeded)
    b = tables(os.path.join(rshome, "bagholder.db"))
    bad = 0
    for t in sorted(set(a) | set(b)):
        if t not in a or t not in b:
            print(f"  table {t}: only in {'python' if t in a else 'rust'}")
            bad += 1
            continue
        if a[t][0] != b[t][0]:
            print(f"  {t}: columns differ\n   py={a[t][0]}\n   rs={b[t][0]}")
            bad += 1
            continue
        if a[t][1] != b[t][1]:
            for i, (x, y) in enumerate(zip(a[t][1], b[t][1])):
                if x != y:
                    print(f"  {t}[{i}]:\n   py={x}\n   rs={y}")
                    bad += 1
                    if bad > 10:
                        break
            if len(a[t][1]) != len(b[t][1]):
                print(f"  {t}: {len(a[t][1])} rows in python, {len(b[t][1])} in rust")
                bad += 1
    rows = sum(len(v[1]) for v in a.values())
    print(f"{len(a)} tables, {rows} rows, {bad} differences")
    shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
