"""Which source prices what, and the quote parsers, both ways -- then a live
refresh of the book's own held symbols into two stores, compared.

    cargo build -p bagholder-market && python3 crates/market/quotetest.py
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

BIN = os.path.join(ROOT, "target", "debug", "quotetool")

RECORDS = [
    {"symbol": "QNC", "exchange": "TSX", "currency": "CAD", "kind": "Shares"},
    {"symbol": "QNC.TO", "exchange": "TSX", "currency": "CAD", "kind": "Shares"},
    {"symbol": "AAPL", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares"},
    {"symbol": "ABC", "exchange": "CBOE CANADA", "currency": "CAD", "kind": "Shares"},
    {"symbol": "DEF", "exchange": "CSE", "currency": "CAD", "kind": "Shares"},
    {"symbol": "BTC", "currency": "CAD", "kind": "Crypto"},
    {"symbol": "ETH", "currency": "USD", "kind": "Crypto"},
    {"symbol": "QNC 20NOV26 3.00 CALL", "currency": "USD", "kind": "Options"},
    {"symbol": "QNC 20NOV26 3.00 CALL", "currency": "CAD", "kind": "Options"},
    {"symbol": "SPX", "exchange": "Index", "currency": "USD", "kind": "Instrument", "yahoo": "^GSPC"},
    {"symbol": "SPX", "exchange": "Index", "currency": "USD", "kind": "Instrument"},
    {"symbol": "", "kind": "Shares"},
    {"symbol": "XYZ", "exchange": "LSE", "currency": "GBP", "kind": "Shares"},
    {"symbol": "XYZ", "kind": "Bond"},
]

YAHOO = [
    json.dumps({"chart": {"result": [{"meta": {
        "regularMarketPrice": 231.4, "chartPreviousClose": 229.0, "currency": "USD",
        "shortName": "Apple Inc.", "exchangeName": "NMS"}}]}}),
    json.dumps({"chart": {"result": [{"meta": {
        "regularMarketPrice": 10.0, "previousClose": 0, "longName": "Thing"}}]}}),
    json.dumps({"chart": {"result": [{"meta": {"regularMarketPrice": None}}]}}),
    json.dumps({"chart": {"result": []}}),
    "{}", "",
]

ROOTS = ["QNC", "BRK.B", "qnc.to", ""]
TMX_QUOTE_SYMBOLS = [["QNC", "TSX", "CAD"], ["QNC", "NASDAQ", "USD"], ["QNC", "CSE", "CAD"],
                     ["A B", "TSX", "CAD"], ["QNC", "LSE", "GBP"]]


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
    payload = {"records": RECORDS, "yahoo": YAHOO, "roots": ROOTS, "tmxQuoteSymbols": TMX_QUOTE_SYMBOLS}
    got = json.loads(subprocess.run([BIN, "pure"], input=json.dumps(payload),
                                    capture_output=True, text=True, check=True).stdout)
    bad = []
    for i, r in enumerate(RECORDS):
        w = market.quote_source(r)
        w = list(w) if w else None
        if norm(w) != norm(got["sources"][i]):
            bad.append(f"source[{i}] {r.get('symbol')}/{r.get('kind')}: py={w} rs={got['sources'][i]}")
        if norm(market.yahoo_forms(r)) != norm(got["forms"][i]):
            bad.append(f"forms[{i}]: py={market.yahoo_forms(r)} rs={got['forms'][i]}")
    for i, t in enumerate(YAHOO):
        if norm(market.parse_yahoo_quote(t)) != norm(got["yahoo"][i]):
            bad.append(f"yahoo[{i}]: py={market.parse_yahoo_quote(t)} rs={got['yahoo'][i]}")
    for i, s in enumerate(ROOTS):
        if market.yahoo_root(s) != got["roots"][i]:
            bad.append(f"root[{i}] {s!r}: py={market.yahoo_root(s)!r} rs={got['roots'][i]!r}")
    for i, (s, e, c) in enumerate(TMX_QUOTE_SYMBOLS):
        if market.tmx_quote_symbol(s, e, c) != got["tmxQuoteSymbols"][i]:
            bad.append(f"tmxQuoteSymbol[{i}]: py={market.tmx_quote_symbol(s, e, c)!r} rs={got['tmxQuoteSymbols'][i]!r}")

    # a live refresh of a few real listings, into two fresh stores
    work = tempfile.mkdtemp(prefix="quotetest-")
    pyhome, rshome = os.path.join(work, "py"), os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)
    import store
    import model
    today, now = model.today_local(), time.time()
    stamp = "2026-09-16T12:00:00Z"
    for home in (pyhome, rshome):
        store.set_home(home)
        store.ensure()
        store.close_all()

    live = [
        {"symbol": "QNC", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "quoteKey": "QNC@TSX"},
        {"symbol": "AAPL", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares", "quoteKey": "AAPL@NASDAQ"},
        {"symbol": "BTC", "currency": "USD", "kind": "Crypto", "quoteKey": "BTC@"},
    ]
    store.set_home(pyhome)
    py_written = market.refresh_quotes(live, None)
    py_quotes = store.quotes()
    store.close_all()

    rs = json.loads(subprocess.run(
        [BIN, "live"],
        input=json.dumps({"db": os.path.join(rshome, "bagholder.db"), "today": today,
                          "now": now, "stamp": stamp, "symbols": live}),
        capture_output=True, text=True, check=True).stdout)

    if py_written != rs["written"]:
        bad.append(f"live: python wrote {py_written} quotes, rust wrote {rs['written']}")
    for key in sorted(set(py_quotes) | set(rs["quotes"])):
        if key not in py_quotes:
            bad.append(f"live quote {key}: only rust")
        elif key not in rs["quotes"]:
            bad.append(f"live quote {key}: only python")
        else:
            # the price itself moves between the two calls; the source and the
            # shape are what must agree
            for f in ("source", "currency", "dividendFrequency"):
                if py_quotes[key].get(f) != rs["quotes"][key].get(f):
                    bad.append(f"live quote {key}.{f}: py={py_quotes[key].get(f)!r} rs={rs['quotes'][key].get(f)!r}")
    shutil.rmtree(work, ignore_errors=True)

    for line in bad[:20]:
        print("  " + line)
    n = len(RECORDS) * 2 + len(YAHOO) + len(ROOTS) + len(TMX_QUOTE_SYMBOLS) + len(live)
    print(f"{n} quote cases ({len(live)} live), {len(bad)} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
