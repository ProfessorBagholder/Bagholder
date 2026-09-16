"""The activity row layer, both ways.

Rows in every shape the app inserts -- broker rows with real ids, broker rows
whose id is homemade, typed-in rows, CSV rows, rows missing dates, rows with
either spelling of every field -- are inserted through both implementations
into their own database, and the stored rows and the readback are compared.
The match keys and the canonical-id reading are compared on the same rows.

    cargo build -p bagholder-store && python3 crates/store/activitytest.py
"""
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
BIN = os.path.join(ROOT, "target", "debug", "storetool")


def rows(n=250):
    rng = random.Random(3)
    out = []
    for i in range(n):
        shape = rng.choice(["ws", "ws", "ws-homemade", "manual", "csv", "snake", "sparse"])
        day = "2026-%02d-%02d" % (rng.randint(1, 12), rng.randint(1, 28))
        base = {
            "id": {"ws": f"ws-{i}", "ws-homemade": f"manual-{i}", "manual": f"a|b|{i}",
                   "csv": "", "snake": f"ws-s-{i}", "sparse": f"ws-p-{i}"}[shape],
            "symbol": rng.choice(["AAA", "ZZZ 21AUG26 10.00 CALL", "ETH", " bbb ", ""]),
            "currency": rng.choice(["CAD", "USD", ""]),
            "quantity": rng.choice([0, 1, -2, 3.14159265358979, 100, -0.000000005]),
            "commission": rng.choice([0, 1.5, None]),
            "category": rng.choice(["trade", "other", "dividend", ""]),
            "accountType": rng.choice(["Trading", "TFSA", ""]),
            "description": rng.choice(["", "a note"]),
            "direction": rng.choice(["", "CREDIT", "DEBIT"]),
            "name": rng.choice(["", "Thing Inc"]),
            "rawType": rng.choice(["DIY_BUY", "OPTIONS_SELL", "DIVIDEND", ""]),
            "activityType": rng.choice(["Trade", "DIVIDEND", ""]),
            "activitySubType": rng.choice(["BUY", "SELL", "SELLTOOPEN", "", "LIMIT_ORDER"]),
            "accountId": rng.choice(["acct-1", "~invented", "manual", "", "CAD"]),
            "balance": rng.choice([None, 0, 1234.56]),
            "securityId": rng.choice(["", "sec-1", "  "]),
            "source": {"ws": "wealthsimple", "ws-homemade": "wealthsimple", "manual": "manual",
                       "csv": "csv", "snake": "wealthsimple", "sparse": "wealthsimple"}[shape],
        }
        if shape == "snake":
            # the other spelling of every field the readers accept
            base = {
                "id": base["id"], "symbol": base["symbol"], "currency": base["currency"],
                "quantity": base["quantity"], "commission": base["commission"],
                "category": base["category"], "account_type": base["accountType"],
                "raw_type": base["rawType"], "activity_type": base["activityType"],
                "activity_sub_type": base["activitySubType"], "account_id": base["accountId"],
                "security_id": base["securityId"], "source": base["source"],
                "transaction_date": day, "occurred_at": day + "T15:00:00+00:00",
                "unit_price": rng.choice([0, 1.25, 100]), "net_cash_amount": rng.choice([0, -125.0, 500]),
                "book_id": "", "fifo_id": "", "settlement_date": "",
            }
            out.append(base)
            continue
        base["unitPrice"] = rng.choice([0, 1.25, 100, None])
        base["netCashAmount"] = rng.choice([0, -125.0, 500, None])
        if shape == "sparse":
            # no transaction date: it has to come off the instant
            base["occurredAt"] = day + "T15:00:00+00:00"
        elif shape == "csv":
            # date-only source stays date-only
            base["transactionDate"] = day
            base["occurredAt"] = day
        else:
            base["transactionDate"] = day
            base["occurredAt"] = day + "T%02d:00:00+00:00" % rng.randint(0, 23)
        out.append(base)
    return out


def run(mode, db, payload=None):
    args = [BIN, mode] + ([db] if db else [])
    r = subprocess.run(args, input=json.dumps(payload) if payload is not None else None,
                       capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stderr[-2000:])
        raise SystemExit(f"storetool {mode} failed")
    return json.loads(r.stdout) if r.stdout.strip() else None


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


def compare(label, want, got, bad):
    if len(want) != len(got):
        bad.append(f"{label}: {len(want)} in python, {len(got)} in rust")
        return
    for i, (w, g) in enumerate(zip(want, got)):
        w, g = norm(w), norm(g)
        if isinstance(w, dict):
            for k in sorted(set(w) | set(g)):
                if w.get(k) != g.get(k):
                    bad.append(f"{label}[{i}].{k}: py={w.get(k)!r} rs={g.get(k)!r}")
        elif w != g:
            bad.append(f"{label}[{i}]: py={w!r} rs={g!r}")


def main():
    work = tempfile.mkdtemp(prefix="activitytest-")
    pyhome = os.path.join(work, "py")
    rshome = os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)

    import store
    store.set_home(pyhome)
    store.ensure()
    store.close_all()
    shutil.copy(os.path.join(pyhome, "bagholder.db"), os.path.join(rshome, "bagholder.db"))
    rsdb = os.path.join(rshome, "bagholder.db")

    data = rows()
    bad = []

    # the match keys and the canonical-id reading, on the rows as given
    want_keys = []
    for a in data:
        source = store._s(a.get("source")) or "wealthsimple"
        want_keys.append({
            "fieldKey": list(store.field_match_key(a, True)),
            "fieldKeyNoAccount": list(store.field_match_key(a, False)),
            "linkKey": list(store.link_match_key(a, True)),
            "tradeSide": store.trade_side(a),
            "homemade": store.looks_like_homemade_id(a.get("id")),
            "realAccount": store.is_real_account(a.get("accountId") or a.get("account_id")),
            "canonical": store._canonical_from_row(a, source),
        })
    compare("keys", want_keys, run("keys", None, data), bad)

    # insertion: each row stored and read back
    counter = {"n": 0}

    def gen():
        i = counter["n"]
        counter["n"] += 1
        return f"gen-{i}"

    real_new_id = store._new_id
    store._new_id = gen
    try:
        want_ins = [store.insert_activity(a) for a in data]
    finally:
        store._new_id = real_new_id
    store.close_all()
    compare("insert", want_ins, run("insert", rsdb, data), bad)

    # and the whole table as the model reads it
    store.set_home(pyhome)
    with store._lock:
        conn = store._connect()
        try:
            want_rows = store._all_activities(conn)
        finally:
            conn.close()
    store.close_all()
    compare("rows", want_rows, run("rows", rsdb), bad)

    for line in bad[:25]:
        print("  " + line)
    print(f"{len(data)} rows in, {len(want_rows)} stored, {len(bad)} differences")
    shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
