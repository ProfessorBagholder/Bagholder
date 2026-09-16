"""Order tickets and brackets, both ways.

Tickets are written before anything is sent, so what is stored has to be right
even when the ticket is odd: no price on a market order, a stop with a trail,
a bracket that has not armed, a patch naming one field. Both implementations
write the same tickets into their own store and the tables and the readbacks
are compared.

    cargo build -p bagholder-store && python3 crates/store/ordertest.py
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

NOW = "2026-09-16T12:00:00Z"

ORDERS = [
    {"id": "o1", "accountId": "a1", "account": "Trading", "securityId": "sec-1", "symbol": "AAA",
     "currency": "CAD", "side": "BUY", "type": "LIMIT", "quantity": 100, "limitPrice": 10.5,
     "tif": "DAY", "status": "draft",
     "request": {"z": 1, "a": {"n": 2, "m": 3}, "k": [1, 2]},
     "stopLoss": {"kind": "stop", "price": 9.0}, "takeProfit": {"price": 12.0}},
    {"id": "o2", "accountId": "a1", "securityId": "sec-2", "symbol": "BBB", "side": "SELL",
     "type": "MARKET", "quantity": 50, "status": "submitted", "wsOrderId": "ws-9",
     "createdAt": "2026-09-15T09:00:00Z", "source": "", "role": ""},
    {"id": "o3", "accountId": "a1", "securityId": "sec-3", "side": "BUY", "type": "STOP_LIMIT",
     "quantity": 5, "limitPrice": None, "stopPrice": 3.25, "status": "working",
     "parentId": "o1", "role": "exit", "tif": "GTC", "request": None, "stopLoss": None},
    {"id": "o4", "accountId": "a1", "securityId": "sec-4", "side": "BUY", "type": "LIMIT",
     "quantity": 0, "status": "", "filledQty": 0, "avgFill": None},
]

ORDER_PATCHES = [
    ["o1", {"status": "submitted", "wsOrderId": "ws-1", "submittedAt": "2026-09-16T12:00:05Z"}],
    ["o1", {"filledQty": 100, "avgFill": 10.49, "wsStatus": "FILLED"}],
    ["o2", {"error": "rejected by the broker"}],
    ["o3", {}],
    ["o3", {"limitPrice": None}],
    ["nope", {"status": "gone"}],
]

BRACKETS = [
    {"id": "b1", "orderId": "o1", "accountId": "a1", "securityId": "sec-1", "symbol": "AAA",
     "currency": "CAD", "quantity": 100, "slKind": "stop", "slPrice": 9.0, "tpPrice": 12.0,
     "slNative": True, "attempts": 0},
    {"id": "b2", "orderId": "o2", "accountId": "a1", "securityId": "sec-2", "quantity": 50,
     "slKind": "trail", "slTrail": 5, "slTrailUnit": "", "tif": "", "status": "", "slNative": False},
    {"id": "b3", "orderId": "o3", "accountId": "a1", "securityId": "sec-3", "quantity": None,
     "status": "armed", "attempts": 3, "highWater": 11.25},
]

BRACKET_PATCHES = [
    ["b1", {"status": "armed", "armedAt": NOW, "attempts": 1}],
    ["b1", {"slPrice": 9.5, "highWater": 10.75, "seenHeld": True}],
    ["b2", {"outcome": "stopped", "status": "done", "slNative": False}],
    ["b3", {"quantity": None}],
    ["b3", {}],
]


def tables(path):
    c = sqlite3.connect(path)
    out = {}
    for t in ("orders", "brackets"):
        cols = [r[1] for r in c.execute(f"PRAGMA table_info({t})")]
        out[t] = list(c.execute(f"SELECT {', '.join(cols)} FROM {t} ORDER BY id"))
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


def main():
    work = tempfile.mkdtemp(prefix="ordertest-")
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
    for row in ORDERS:
        store.insert_order(row)
    for oid, patch in ORDER_PATCHES:
        store.update_order(oid, patch)
    for b in BRACKETS:
        store.insert_bracket(b)
    for bid, patch in BRACKET_PATCHES:
        store.update_bracket(bid, patch)
    # The answer Python gives here is `conn.total_changes > 0`, which counts
    # every change the pooled connection has made and so reads true even for an
    # order that does not exist. What is compared is the stored quantity, which
    # is what the marker is for; the Rust answer is the accurate one and is
    # asserted separately below.
    booked_py = [store.mark_order_fill_booked("o1", 50), store.mark_order_fill_booked("o1", 100),
                 store.mark_order_fill_booked("o1", 75), store.mark_order_fill_booked("nope", 1)]
    want = {
        "orders": store.list_orders(200),
        # replaced below; see the note on booked_py
        "booked": [True, True, True, True],
        "brackets": store.list_brackets(),
        "byStatus": store.list_brackets(["armed", "done"]),
        "forOrder": store.bracket_for_order("o1"),
        "symbolFor": store.symbol_for_security("sec-1"),
    }
    store.close_all()

    payload = {"now": NOW, "orders": ORDERS, "orderPatches": ORDER_PATCHES,
               "brackets": BRACKETS, "bracketPatches": BRACKET_PATCHES,
               "booked": [["o1", 50], ["o1", 100], ["o1", 75], ["nope", 1]],
               "statuses": ["armed", "done"], "forOrder": "o1", "symbolFor": "sec-1"}
    r = subprocess.run([BIN, "orders", os.path.join(rshome, "bagholder.db")],
                       input=json.dumps(payload), capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stderr[-2000:])
        return 1
    got = json.loads(r.stdout)

    bad = []
    # the accurate answers: a growing quantity changes a row, a smaller one and
    # an unknown order do not
    if got["booked"] != [True, True, False, False]:
        bad.append(f"booked: rust answered {got['booked']}, expected [True, True, False, False]")
    want.pop("booked", None)
    got.pop("booked", None)
    for key in want:
        if norm(want[key]) != norm(got[key]):
            w, g = norm(want[key]), norm(got[key])
            if isinstance(w, list) and isinstance(g, list) and len(w) == len(g):
                for i, (x, y) in enumerate(zip(w, g)):
                    if x != y:
                        if isinstance(x, dict):
                            for k in sorted(set(x) | set(y)):
                                if x.get(k) != y.get(k):
                                    bad.append(f"{key}[{i}].{k}: py={x.get(k)!r} rs={y.get(k)!r}")
                        else:
                            bad.append(f"{key}[{i}]: py={x!r} rs={y!r}")
            else:
                bad.append(f"{key}: py={w!r} rs={g!r}")

    a, b = tables(os.path.join(pyhome, "bagholder.db")), tables(os.path.join(rshome, "bagholder.db"))
    for t in ("orders", "brackets"):
        if a[t] != b[t]:
            for x, y in zip(a[t], b[t]):
                if x != y:
                    bad.append(f"table {t}:\n    py={x}\n    rs={y}")
                    break
            if len(a[t]) != len(b[t]):
                bad.append(f"table {t}: {len(a[t])} rows py, {len(b[t])} rs")

    for line in bad[:25]:
        print("  " + line)
    print(f"{len(ORDERS)} tickets, {len(BRACKETS)} brackets, "
          f"{len(ORDER_PATCHES) + len(BRACKET_PATCHES)} patches, {len(bad)} differences")
    shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
