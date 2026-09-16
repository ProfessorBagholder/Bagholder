"""Accounts, balances, margin, NAV, FX, the benchmark series and the journal,
written and read back through both implementations and compared -- both the
values each returns and the tables each leaves behind.

The rows deliberately include the shapes the cleaners are there to reject:
accounts with no id, non-object rows, NAV points with no date or no equity,
dates that are not dates, non-positive rates, groups with no members and
duplicate ids, notes that are empty or carry a grade outside A/B/C/F.

    cargo build -p bagholder-store && python3 crates/store/tabletest.py
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

NOW = "2026-09-16T00:00:00Z"

PAYLOAD = {
    "now": NOW,
    "accounts": [
        {"id": "acct-1", "nickname": "Trading", "unifiedAccountType": "CASH", "currency": "CAD",
         "status": "open", "type": "SELF_DIRECTED", "netLiquidationValue": 12345.67, "marginAccountId": ""},
        {"id": "acct-2", "nickname": "TFSA", "unified_account_type": "TFSA", "currency": "CAD",
         "status": "open", "type": "MARGIN", "netLiquidationValue": None, "marginAccountId": "m-2"},
        {"id": "", "nickname": "nope"},
        "not a dict",
        {"nickname": "no id either"},
    ],
    "balances": [
        {"accountId": "acct-1", "custodianAccountId": "cu-1", "securityId": "sec-c-cad", "quantity": -250.5},
        {"account_id": "acct-2", "custodian_account_id": "cu-2", "security_id": "sec-1", "quantity": 100},
        {"accountId": "acct-1", "securityId": "sec-2", "quantity": None},
        42,
    ],
    "margin": [
        {"accountId": "acct-2", "buyingPower": 5000.0, "currency": "", "unavailable": "", "fetchedAt": ""},
        {"accountId": "acct-3", "buyingPower": None, "currency": "USD", "unavailable": "not a margin account"},
        {"accountId": "", "buyingPower": 1},
        None,
    ],
    "nav": [
        {"date": "2026-01-02", "equity": 100.0, "currency": "CAD", "netDeposits": 100.0},
        {"date": "2026-01-03T00:00:00Z", "equity": 110.0},
        {"date": "2026-01-04", "equity": 120.0, "accountId": "acct-1", "netDeposits": None},
        {"date": "", "equity": 1},
        {"date": "2026-01-05", "equity": None},
        {"date": "2026-01-06", "equity": 130.0, "account_id": "acct-2", "net_deposits": 5},
        "junk",
    ],
    "navUpsert": [
        {"date": "2026-01-02", "equity": 101.0, "netDeposits": 100.0},
        {"date": "2026-01-07", "equity": 140.0},
    ],
    "fx": {"2026-01-02": 1.37, "2026-01-03": 1.36, "bad": 1.0, "2026-01-04": 0,
           "2026-01-05": -1, "2026-01-06": None, "2026-01-07": "1.35"},
    "benchmark": {"2026-01-02": 6800.0, "2026-01-0": 1, "2026-01-03": 6850.5},
    "groups": [
        {"id": "g1", "locked": True, "members": ["a|b|1.0", "c|d|2.0", "a|b|1.0"]},
        {"id": "g1", "members": ["dup id ignored"]},
        {"id": "", "members": ["x"]},
        {"id": "g2", "members": []},
        {"id": "g3", "members": "not a list"},
        {"id": "g4", "locked": 0, "members": [" e|f|3.0 ", ""]},
        "junk",
    ],
    "notes": {
        "rt:1": {"thesis": "Breakout.", "tag": "earnings", "grade": "A"},
        "rt:2": {"thesis": "", "tag": "", "grade": ""},
        "rt:3": {"grade": "Z"},
        "rt:4": {"grade": "F"},
        " ": {"grade": "A"},
        "rt:5": "not a dict",
    },
}


def python_side(home):
    import store
    store._now_iso = lambda: NOW      # margin stamps the time it was read
    store.set_home(home)
    store.ensure()
    p = PAYLOAD
    store.replace_accounts(p["accounts"])
    store.replace_balances(p["balances"])
    store.replace_margin(p["margin"])
    store.replace_nav(p["nav"])
    store.upsert_nav(p["navUpsert"])
    fx_n = store.upsert_fx_rates(p["fx"])
    bench_n = store.upsert_benchmark_prices(p["benchmark"])
    saved_groups = store.save_trade_groups(p["groups"])
    saved_notes = store.save_trade_notes(p["notes"])

    with store._lock:
        conn = store._connect()
        try:
            def rows(sql, args=()):
                return [dict(r) for r in conn.execute(sql, args).fetchall()]
            accounts = rows("SELECT id, nickname, unified_account_type, currency, status, type, "
                            "net_liquidation_value, margin_account_id FROM accounts ORDER BY id")
            balances = rows("SELECT account_id, custodian_account_id, security_id, quantity FROM balances ORDER BY id")
            margin = rows("SELECT account_id, buying_power, currency, unavailable, fetched_at FROM margin ORDER BY account_id")
            nav_all = [store._nav_point_from_row(r) for r in conn.execute(
                "SELECT date, equity, currency, net_deposits FROM nav_history WHERE account_id = '' ORDER BY date")]
        finally:
            conn.close()

    out = {
        "fxWritten": fx_n, "benchWritten": bench_n,
        "savedGroups": saved_groups, "savedNotes": saved_notes,
        "accounts": [{"id": a["id"], "nickname": a["nickname"],
                      "unifiedAccountType": a["unified_account_type"], "currency": a["currency"],
                      "status": a["status"], "type": a["type"],
                      "netLiquidationValue": a["net_liquidation_value"],
                      "marginAccountId": a["margin_account_id"]} for a in accounts],
        "balances": [{"accountId": b["account_id"], "custodianAccountId": b["custodian_account_id"],
                      "securityId": b["security_id"], "quantity": b["quantity"]} for b in balances],
        "margin": [{"accountId": m["account_id"], "buyingPower": m["buying_power"],
                    "currency": m["currency"], "unavailable": m["unavailable"],
                    "fetchedAt": m["fetched_at"]} for m in margin],
        "navAll": nav_all,
        "navLastDates": store.nav_last_dates(),
        "fx": store.fx_rates(),
        "fxLast": store.fx_last_date(),
        "bench": store.benchmark_prices(),
        "benchLast": store.benchmark_last_date(),
        "benchDays": store.benchmark_days(store.BENCHMARK_SYMBOL, "2024-01-01", "2026-12-31"),
        "groups": store.trade_groups(),
        "notes": store.trade_notes(),
    }
    store.close_all()
    return out


def tables(path):
    c = sqlite3.connect(path)
    names = [r[0] for r in c.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    out = {}
    for t in names:
        cols = [r[1] for r in c.execute(f"PRAGMA table_info({t})")]
        out[t] = list(c.execute(f"SELECT {', '.join(cols)} FROM {t} ORDER BY {', '.join(cols)}"))
    c.close()
    return out


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


def diff(path, want, got, out):
    if isinstance(want, dict) and isinstance(got, dict):
        for k in sorted(set(want) | set(got)):
            if k not in want:
                out.append(f"{path}.{k}: only rust ({got[k]!r})")
            elif k not in got:
                out.append(f"{path}.{k}: only python ({want[k]!r})")
            else:
                diff(f"{path}.{k}", want[k], got[k], out)
    elif isinstance(want, list) and isinstance(got, list):
        if len(want) != len(got):
            out.append(f"{path}: {len(want)} in python, {len(got)} in rust")
            return
        for i, (w, g) in enumerate(zip(want, got)):
            diff(f"{path}[{i}]", w, g, out)
    elif norm(want) != norm(got):
        out.append(f"{path}: py={want!r} rs={got!r}")


def main():
    work = tempfile.mkdtemp(prefix="tabletest-")
    pyhome = os.path.join(work, "py")
    rshome = os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)

    want = python_side(pyhome)

    # the Rust side starts from the same empty schema
    import store
    store.set_home(rshome)
    store.ensure()
    store.close_all()
    rsdb = os.path.join(rshome, "bagholder.db")
    r = subprocess.run([BIN, "tables", rsdb], input=json.dumps(PAYLOAD), capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stderr[-2000:])
        return 1
    got = json.loads(r.stdout)

    bad = []
    diff("", want, got, bad)

    a, b = tables(os.path.join(pyhome, "bagholder.db")), tables(rsdb)
    for t in sorted(set(a) | set(b)):
        if a.get(t) != b.get(t):
            bad.append(f"table {t}: py={a.get(t)!r} rs={b.get(t)!r}")

    for line in bad[:25]:
        print("  " + line)
    print(f"{len(a)} tables, {len(bad)} differences")
    shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
