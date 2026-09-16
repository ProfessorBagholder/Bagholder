"""Run every case's snapshot through both build_book implementations and
compare the normalized activities (including the assignment shares and the
assumed expiries neither broker posts), the FIFO result after apply_fx, and
the securities readings.

    cargo build --bin difftool && python3 crates/model/booktest.py
"""
import glob
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import model  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "difftool")


def norm(v):
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return round(float(v), 9) + 0.0
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, list):
        return [norm(x) for x in v]
    return v


def main():
    cases = sorted(glob.glob(os.path.join(ROOT, "tests", "cases", "*.json")))
    bad = []
    counts = {"activities": 0, "closed": 0, "open": 0}
    for path in cases:
        name = os.path.basename(path)[:-5]
        doc = json.load(open(path))
        snap, today = doc["snapshot"], doc["today"]
        fx = doc["market"].get("fx") or {}

        book = model.build_book(snap, today)
        model.apply_fx(book["fifo"]["closed"], fx)
        want = {
            "activities": book["activities"],
            "closed": book["fifo"]["closed"],
            "open": book["fifo"]["open"],
            "unmatched": book["fifo"]["unmatched"],
            "rawCount": book["rawCount"],
            "cashCurrencies": book["securities"].cash_currencies(),
            "knownExchanges": book["securities"].known_exchanges(),
        }
        got = json.loads(subprocess.run(
            [BIN, "book"], input=json.dumps({"snapshot": snap, "today": today, "fx": fx}),
            capture_output=True, text=True, check=True).stdout)

        for k in ("rawCount", "cashCurrencies", "knownExchanges"):
            if norm(want[k]) != norm(got[k]):
                bad.append(f"{name}: {k} py={want[k]!r} rs={got[k]!r}")
        for section in ("activities", "closed", "open", "unmatched"):
            w, g = norm(want[section]), norm(got[section])
            counts[section] = counts.get(section, 0) + len(w)
            if len(w) != len(g):
                bad.append(f"{name}: {section} has {len(w)} in python, {len(g)} in rust")
                continue
            for i, (wr, gr) in enumerate(zip(w, g)):
                for key in sorted(set(wr) | set(gr)):
                    if wr.get(key) != gr.get(key):
                        bad.append(f"{name}: {section}[{i}].{key} py={wr.get(key)!r} rs={gr.get(key)!r}")
    print(f"{len(cases)} cases; " + ", ".join(f"{v} {k}" for k, v in counts.items()))
    for line in bad[:40]:
        print("  " + line)
    print(f"{len(bad)} mismatches")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
