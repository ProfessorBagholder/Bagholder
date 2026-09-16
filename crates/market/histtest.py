"""The daily-bar chain, both ways: which sources an instrument's bars can come
from, the currency a candidate is quoted in, the conversion into the
position's currency, the weekly and monthly aggregation, Yahoo's chart parser
-- and then a live fetch of a real listing into two stores.

    cargo build -p bagholder-market && python3 crates/market/histtest.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import market  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "histtool")

RECORDS = [
    {"symbol": "QNC", "exchange": "TSX", "currency": "CAD", "kind": "Shares"},
    {"symbol": "AAPL", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares"},
    {"symbol": "BTC", "currency": "CAD", "kind": "Crypto"},
    {"symbol": "ETH", "currency": "USD", "kind": "Crypto"},
    {"symbol": "QNC 20NOV26 3.00 CALL", "currency": "USD", "kind": "Options"},
    {"symbol": "", "kind": "Shares"},
    {"symbol": "XYZ", "exchange": "LSE", "currency": "GBP", "kind": "Shares"},
]

BAR_CURRENCY = [
    ["coinbase", "BTC-CAD", {"kind": "Crypto", "currency": "CAD"}],
    ["coinbase", "BTC-USD", {"kind": "Crypto", "currency": "CAD"}],
    ["tmx", "QNC", {"kind": "Shares", "currency": "CAD"}],
    ["yahoo", "AAPL", {"kind": "Shares", "currency": "USD"}],
    ["yahoo", "AAPL", {"kind": "Shares"}],
]

FX = {"2026-09-14": 1.36, "2026-09-15": 1.37}
BARS_USD = [
    {"date": "2026-09-15", "open": 10.0, "high": 11.0, "low": 9.0, "close": 10.5, "volume": 100},
    {"date": "2026-09-14", "open": 9.0, "high": 10.0, "low": 8.5, "close": 9.5, "volume": 50},
    {"date": "2026-08-01", "open": 1.0, "high": 2.0, "low": 0.5, "close": 1.5, "volume": 10},
]
CONVERT = [
    {"bars": BARS_USD, "quoted": "USD", "currency": "CAD"},
    {"bars": BARS_USD, "quoted": "CAD", "currency": "CAD"},
    {"bars": BARS_USD, "quoted": "GBP", "currency": "CAD"},
    {"bars": BARS_USD, "quoted": "USD", "currency": ""},
]

DAILY = [
    {"date": "2026-09-07", "open": 1.0, "high": 2.0, "low": 0.5, "close": 1.5, "volume": 10},
    {"date": "2026-09-08", "open": 1.5, "high": 3.0, "low": 1.4, "close": 2.5, "volume": 20},
    {"date": "2026-09-14", "open": 2.5, "high": 2.6, "low": 2.0, "close": 2.2, "volume": 5},
    {"date": "2026-10-01", "open": 2.2, "high": 4.0, "low": 2.1, "close": 3.9, "volume": 7},
    {"date": "2026-10-02", "open": None, "high": None, "low": None, "close": 4.0, "volume": None},
]
AGGREGATE = [{"bars": DAILY, "tf": "1w"}, {"bars": DAILY, "tf": "1M"}, {"bars": [], "tf": "1w"}]

YAHOO_CHARTS = [
    json.dumps({"chart": {"result": [{
        "timestamp": [1789000000, 1789086400],
        "indicators": {"quote": [{"open": [1.0, 2.0], "high": [1.5, 2.5], "low": [0.9, 1.9],
                                  "close": [1.2, 2.2], "volume": [10, 20]}]},
        "meta": {"exchangeTimezoneName": "America/New_York", "gmtoffset": -14400}}]}}),
    json.dumps({"chart": {"result": [{
        "timestamp": [1789000000], "indicators": {"quote": [{"close": [0]}]},
        "meta": {"gmtoffset": 0}}]}}),
    json.dumps({"chart": {"result": []}}), "{}", "",
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
    payload = {"records": RECORDS, "barCurrency": BAR_CURRENCY, "fx": FX,
               "convert": CONVERT, "aggregate": AGGREGATE, "yahooCharts": YAHOO_CHARTS}
    got = json.loads(subprocess.run([BIN, "pure"], input=json.dumps(payload),
                                    capture_output=True, text=True, check=True).stdout)
    bad = []
    for i, r in enumerate(RECORDS):
        w = [list(c) for c in market.history_candidates(r)]
        if norm(w) != norm(got["candidates"][i]):
            bad.append(f"candidates[{i}] {r.get('symbol')}: py={w} rs={got['candidates'][i]}")
    for i, (s, k, rec) in enumerate(BAR_CURRENCY):
        if market.bar_currency(s, k, rec) != got["barCurrency"][i]:
            bad.append(f"barCurrency[{i}]: py={market.bar_currency(s, k, rec)!r} rs={got['barCurrency'][i]!r}")

    # the conversion reads the stored rates on the Python side
    work = tempfile.mkdtemp(prefix="histtest-")
    pyhome, rshome = os.path.join(work, "py"), os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)
    import store
    import model
    for home in (pyhome, rshome):
        store.set_home(home)
        store.ensure()
        store.upsert_fx_rates(FX)
        store.close_all()
    store.set_home(pyhome)
    for i, c in enumerate(CONVERT):
        w = market.in_position_currency(c["bars"], c["quoted"], c["currency"])
        if norm(w) != norm(got["convert"][i]):
            bad.append(f"convert[{i}]: py={w} rs={got['convert'][i]}")
    store.close_all()

    for i, a in enumerate(AGGREGATE):
        w = market.aggregate_daily(a["bars"], a["tf"])
        if norm(w) != norm(got["aggregate"][i]):
            bad.append(f"aggregate[{i}] {a['tf']}: py={w} rs={got['aggregate'][i]}")
    for i, t in enumerate(YAHOO_CHARTS):
        w = market.parse_yahoo_chart(t)
        if norm(w) != norm(got["yahooCharts"][i]):
            bad.append(f"yahooChart[{i}]: py={w} rs={got['yahooCharts'][i]}")

    # one live listing's daily bars, into each store
    today = model.today_local()
    start = market.tmx_symbol and (model.shift_date(today, -120))
    live = [{"symbol": "QNC", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}]
    store.set_home(pyhome)
    want_live = [{"symbol": r["symbol"], "bars": market.ensure_history(r, start, today)} for r in live]
    store.close_all()
    got_live = json.loads(subprocess.run(
        [BIN, "live"],
        input=json.dumps({"db": os.path.join(rshome, "bagholder.db"), "today": today,
                          "now": time.time(), "stamp": "2026-09-16T12:00:00Z",
                          "start": start, "end": today, "records": live}),
        capture_output=True, text=True, check=True).stdout)
    for i, (w, g) in enumerate(zip(want_live, got_live)):
        wb, gb = w["bars"], g["bars"]
        if len(wb) != len(gb):
            bad.append(f"live[{i}]: {len(wb)} bars py, {len(gb)} rs")
        else:
            for j, (x, y) in enumerate(zip(wb, gb)):
                if norm(x) != norm(y):
                    bad.append(f"live[{i}].bars[{j}]: py={x} rs={y}")
                    break
    shutil.rmtree(work, ignore_errors=True)

    for line in bad[:20]:
        print("  " + line)
    n = len(RECORDS) + len(BAR_CURRENCY) + len(CONVERT) + len(AGGREGATE) + len(YAHOO_CHARTS) + len(live)
    print(f"{n} history cases ({len(live)} live, {len(want_live[0]['bars'])} bars), {len(bad)} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
