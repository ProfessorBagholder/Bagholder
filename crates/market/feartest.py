"""The fear-and-greed readers, both ways: the scale's own words, both
publishers' answers on fixtures, and then the live readings.

    cargo build -p bagholder-market && python3 crates/market/feartest.py
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import fear  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "feartool")

BANDS = [0, 24.9, 25, 44.9, 45, 55, 55.9, 56, 75.9, 76, 100, None]
RATINGS = [["extreme fear", 10], ["GREED", 80], ["", 50], ["  ", None], ["Neutral", 50], ["", None]]

STOCKS = [
    {"fear_and_greed": {"score": 42.37, "rating": "fear", "timestamp": "2026-09-16T16:00:00-04:00",
                        "previous_close": 44.1, "previous_1_week": 55.0,
                        "previous_1_month": 61.2, "previous_1_year": None},
     "market_momentum_sp125": {"score": 30.5, "rating": "extreme fear"},
     "market_momentum_sp": {"score": 99, "rating": "ignored"},
     "stock_price_strength": {"score": 55.0, "rating": ""},
     "put_call_options": {"score": None},
     "market_volatility_vix_50": {"score": 70.0, "rating": "greed"},
     "fear_and_greed_historical": {"data": [
         {"x": 1789000000000, "y": 40.0}, {"x": 1788913600000, "y": 44.0},
         {"x": None, "y": 1}, {"x": 1788000000000, "y": None}]}},
    {"fear_and_greed": {"score": None}},
    {},
]

CRYPTO = [
    {"data": [
        {"timestamp": "1789000000", "value": "55", "value_classification": "Greed"},
        {"timestamp": 1788913600, "value": 48, "value_classification": "neutral"},
        {"timestamp": None, "value": 1},
        {"timestamp": 1788000000, "value": None},
    ]},
    {"data": []},
    {},
]


def norm(v):
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return round(float(v), 6) + 0.0
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [norm(x) for x in v]
    return v


def main():
    payload = {"stocks": STOCKS, "crypto": CRYPTO, "bands": BANDS, "ratings": RATINGS}
    got = json.loads(subprocess.run([BIN, "pure"], input=json.dumps(payload),
                                    capture_output=True, text=True, check=True).stdout)
    bad = []
    for i, s in enumerate(BANDS):
        if fear.band(s) != got["bands"][i]:
            bad.append(f"band[{i}] {s}: py={fear.band(s)!r} rs={got['bands'][i]!r}")
    for i, (g, s) in enumerate(RATINGS):
        if fear.rating(g, s) != got["ratings"][i]:
            bad.append(f"rating[{i}] {g!r}/{s}: py={fear.rating(g, s)!r} rs={got['ratings'][i]!r}")
    for i, d in enumerate(STOCKS):
        if norm(fear.parse_stocks(d)) != norm(got["stocks"][i]):
            bad.append(f"stocks[{i}]:\n    py={json.dumps(norm(fear.parse_stocks(d)))[:300]}\n    rs={json.dumps(norm(got['stocks'][i]))[:300]}")
    for i, d in enumerate(CRYPTO):
        if norm(fear.parse_crypto(d)) != norm(got["crypto"][i]):
            bad.append(f"crypto[{i}]:\n    py={json.dumps(norm(fear.parse_crypto(d)))[:300]}\n    rs={json.dumps(norm(got['crypto'][i]))[:300]}")

    # the live readings: the score moves, so the shape and the source are what
    # must agree
    want_live = [fear.read(i) for i in ("stocks", "crypto")]
    got_live = json.loads(subprocess.run([BIN, "live"], input=json.dumps({"indexes": ["stocks", "crypto"]}),
                                         capture_output=True, text=True, check=True).stdout)
    for i, (w, g) in enumerate(zip(want_live, got_live)):
        if not w:
            print(f"  (the {['stocks', 'crypto'][i]} publisher did not answer; skipped)")
            continue
        for k in ("index", "source"):
            if w.get(k) != g.get(k):
                bad.append(f"live[{i}].{k}: py={w.get(k)!r} rs={g.get(k)!r}")
        if len(w.get("parts", [])) != len(g.get("parts", [])):
            bad.append(f"live[{i}].parts: {len(w.get('parts', []))} py, {len(g.get('parts', []))} rs")
        if abs(len(w.get("series", [])) - len(g.get("series", []))) > 1:
            bad.append(f"live[{i}].series: {len(w.get('series', []))} py, {len(g.get('series', []))} rs")

    for line in bad[:20]:
        print("  " + line)
    n = len(BANDS) + len(RATINGS) + len(STOCKS) + len(CRYPTO) + 2
    print(f"{n} fear cases (2 live), {len(bad)} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
