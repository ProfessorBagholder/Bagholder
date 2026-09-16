"""Random books through both match_fifo implementations.

The thirty-three shared cases are each one situation someone met; this builds
books the cases do not cover -- shares and options and crypto in the same
account, partial exits, shorts, same-day covers, multileg rows with no
quantity, assignments, expiries, renamed tickers -- and compares the two
matchers on each one. A seed that differs is printed so it can be replayed.

    cargo build --bin difftool && python3 crates/model/fuzz.py [runs]
"""
import json
import os
import random
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import model  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "difftool")
SYMBOLS = ["AAA", "BBB", "CCC.U", "ETH", "BTC"]
OPTIONS = ["ZZZ 21AUG26 10.00 CALL", "ZZZ 18SEP26 12.00 CALL", "ZZZ 21AUG26 10.00 PUT", "QQQ 20MAR26 5.00 PUT"]
ACCOUNTS = ["Trading", "TFSA", "Crypto"]
CURRENCIES = ["CAD", "USD"]


def day(rng):
    return "2026-%02d-%02d" % (rng.randint(1, 12), rng.randint(1, 28))


def row(rng, i):
    """One activity in a shape Wealthsimple actually posts."""
    shape = rng.choice(["share", "share", "option", "option", "multileg", "crypto", "expiry", "assign", "corpaction"])
    d = day(rng)
    base = {
        "id": "a%d" % i,
        "accountId": "acct-1",
        "accountType": rng.choice(ACCOUNTS),
        "currency": rng.choice(CURRENCIES),
        "transactionDate": d,
        "occurredAt": d + "T%02d:00:00+00:00" % rng.randint(0, 23),
        "name": "Thing",
        "securityId": "",
        "commission": rng.choice([0, 0, 0, 1.5]),
        "category": "trade",
        "description": "",
    }
    if shape == "share":
        qty = rng.randint(1, 200)
        px = round(rng.uniform(1, 80), 2)
        buy = rng.random() < 0.55
        base.update(symbol=rng.choice(SYMBOLS), activityType="Trade",
                    activitySubType=rng.choice(["BUY", "BUYTOOPEN"]) if buy else rng.choice(["SELL", "SELLTOOPEN", "SELLTOCLOSE"]),
                    rawType="DIY_BUY" if buy else "DIY_SELL",
                    quantity=qty if buy else -qty, unitPrice=px,
                    netCashAmount=round((-1 if buy else 1) * qty * px, 2))
    elif shape == "option":
        qty = rng.randint(1, 5)
        px = round(rng.uniform(0.1, 9), 2)
        sell = rng.random() < 0.5
        base.update(symbol=rng.choice(OPTIONS),
                    activityType="OPTIONS_SELL" if sell else "OPTIONS_BUY",
                    activitySubType=rng.choice(["SELLTOOPEN", "SELLTOCLOSE"]) if sell else rng.choice(["BUYTOCLOSE", "BUYTOOPEN"]),
                    rawType="OPTIONS_SELL" if sell else "OPTIONS_BUY",
                    quantity=-qty if sell else qty, unitPrice=px,
                    netCashAmount=round((1 if sell else -1) * qty * px * 100, 2))
    elif shape == "multileg":
        # as posted: quantity zero, only the cash
        base.update(symbol=rng.choice(OPTIONS), activityType="OPTIONS_MULTILEG",
                    activitySubType="FILLED", rawType="OPTIONS_MULTILEG",
                    quantity=0, unitPrice=0,
                    netCashAmount=round(rng.choice([-1, 1]) * rng.randint(1, 8) * rng.choice([50, 100, 150, 225]), 2))
    elif shape == "crypto":
        kind = rng.choice(["CRYPTO_BUY", "CRYPTO_SELL", "CRYPTO_STAKING_REWARD", "CRYPTO_TRANSFER"])
        qty = round(rng.uniform(0.01, 5), 6)
        px = round(rng.uniform(100, 4000), 2)
        base.update(symbol=rng.choice(["ETH", "BTC"]), accountType="Crypto", currency="CAD",
                    activityType=kind, rawType=kind,
                    activitySubType=rng.choice(["MARKET_ORDER", "TRANSFER_IN", "TRANSFER_OUT", "other"]),
                    direction=rng.choice(["CREDIT", "DEBIT"]),
                    quantity=qty, unitPrice=px, netCashAmount=round(qty * px, 2))
    elif shape == "expiry":
        base.update(symbol=rng.choice(OPTIONS), activityType="OPTIONS_EXPIRY",
                    activitySubType=rng.choice(["EXPIRED", "SHORT_EXPIRED"]),
                    rawType=rng.choice(["OPTIONS_EXPIRY", "OPTIONS_SHORT_EXPIRY"]),
                    quantity=rng.choice([0, rng.randint(1, 4)]), unitPrice=0, netCashAmount=0,
                    category="option_event")
    elif shape == "assign":
        base.update(symbol=rng.choice(OPTIONS), activityType="OPTIONS_ASSIGNMENT",
                    activitySubType="ASSIGNED", rawType="OPTIONS_ASSIGNMENT",
                    quantity=rng.choice([0, rng.randint(1, 4)]), unitPrice=0, netCashAmount=0,
                    category="option_event")
    else:
        base.update(symbol=rng.choice(SYMBOLS), activityType="STKDIS",
                    activitySubType=rng.choice(["BUY", "SELL"]),
                    rawType=rng.choice(["CORPORATE_ACTION", "DIVIDEND"]),
                    quantity=rng.choice([0, rng.randint(1, 50), -rng.randint(1, 50)]),
                    unitPrice=0, netCashAmount=0)
    return base


def book(seed):
    rng = random.Random(seed)
    acts = [row(rng, i) for i in range(rng.randint(2, 18))]
    acts.sort(key=lambda a: a["transactionDate"])
    for i, a in enumerate(acts):
        a["id"] = "a%d" % i
    return acts


def norm(v):
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return round(float(v), 6) + 0.0
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, list):
        return [norm(x) for x in v]
    return v


def compare(acts):
    """The differences between the two matchers on one book, as strings."""
    try:
        want = model.match_fifo([dict(a) for a in acts])
    except Exception as e:                      # a book python itself rejects is not a case
        return ["python raised %s: %s" % (type(e).__name__, e)]
    got = json.loads(subprocess.run([BIN, "fifo"], input=json.dumps(acts),
                                    capture_output=True, text=True, check=True).stdout)
    out = []
    for section in ("closed", "open", "unmatched"):
        w, g = norm(want[section]), norm(got[section])
        if len(w) != len(g):
            out.append("%s: %d rows in python, %d in rust" % (section, len(w), len(g)))
            continue
        for i, (wr, gr) in enumerate(zip(w, g)):
            for k in sorted(set(wr) | set(gr)):
                if wr.get(k) != gr.get(k):
                    out.append("%s[%d].%s py=%r rs=%r" % (section, i, k, wr.get(k), gr.get(k)))
    return out


def main():
    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 500
    failed = 0
    for seed in range(runs):
        diffs = compare(book(seed))
        if diffs:
            failed += 1
            if failed <= 5:
                print("seed %d:" % seed)
                for line in diffs[:6]:
                    print("   " + line)
    print("%d books, %d differing" % (runs, failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
