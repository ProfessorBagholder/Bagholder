import glob, json, os, subprocess, sys
sys.path.insert(0, ".")
import model

rows = []
for p in sorted(glob.glob("tests/cases/*.json")):
    doc = json.load(open(p))
    for a in doc["snapshot"]["activities"]:
        rows.append(a)
# plus a spread of symbol shapes the cases do not carry
for sym in ["", "AAA", "ZZZ 21AUG26 10.00 CALL", "ZZZ 21AUG26 10.00 PUT", "SPY 260821C00450000",
            "SPY 260821P00450000", "BRK.B", "ETH", "AAPL  C", "AAPL P", "X 1JAN26", "TOOLONGSYMBOL 260821C1"]:
    rows.append({"symbol": sym, "rawType": "", "activityType": "", "activitySubType": "",
                 "quantity": 1, "unitPrice": 1.0, "netCashAmount": -1.0, "currency": "CAD",
                 "accountType": "Trading", "transactionDate": "2026-01-01"})

def py(a):
    s = model._s(a.get("symbol"))
    return {
        "normalized": model.normalize_activity(a),
        "kind": model.kind_of(a),
        "isOption": model.is_option_symbol(s),
        "underlying": model.underlying_symbol(s),
        "right": model.option_right(s),
        "mult": float(model.option_multiplier(s)),
        "closeOnly": model.is_close_only(a),
        "open": model.is_intentional_open(a),
        "fifoAccount": model.fifo_account(a),
        "bookKey": model.book_key(a),
        "dirBuy": model.opening_direction(a, "BUY"),
        "dirSell": model.opening_direction(a, "SELL"),
    }

expected = [py(a) for a in rows]
got = json.loads(subprocess.run(["./target/debug/difftool"], input=json.dumps(rows),
                                capture_output=True, text=True, check=True).stdout)

def norm(v):
    if isinstance(v, float) and v == int(v): return int(v)
    if isinstance(v, dict): return {k: norm(x) for k, x in v.items()}
    if isinstance(v, list): return [norm(x) for x in v]
    return v

bad = 0
for i, (e, g) in enumerate(zip(expected, got)):
    for k in e:
        if norm(e[k]) != norm(g.get(k)):
            bad += 1
            print(f"row {i} sym={rows[i].get('symbol')!r} raw={rows[i].get('rawType')!r} key={k}")
            print(f"   py={json.dumps(norm(e[k]))[:200]}")
            print(f"   rs={json.dumps(norm(g.get(k)))[:200]}")
            if bad > 25: sys.exit(1)
print(f"{len(rows)} rows, {bad} mismatches")
