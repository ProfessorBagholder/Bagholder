"""The TMX readers, both ways: the quote and dividend parsers on fixtures, the
venue names, the record symbol per venue, and a live quote for a listing the
book actually holds.

    cargo build -p bagholder-market && python3 crates/market/tmxtest.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import market  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "tmxtool")

QUOTES = [
    {"data": {"getQuoteBySymbol": {
        "symbol": "QNC", "name": "QNC Inc", "exchangeName": "Toronto Stock Exchange",
        "price": 3.21, "priceChange": 0.05, "percentChange": 1.58, "prevClose": 3.16,
        "currency": "CAD", "dividendFrequency": "Quarterly", "dividendAmount": 0.05,
        "exDividendDate": "2026-09-15T00:00:00Z"}}},
    {"data": {"getQuoteBySymbol": {"symbol": "X", "price": None}}},
    {"data": {"getQuoteBySymbol": {}}},
    {"data": {}},
    {},
]

DIVIDENDS = [
    {"data": {"dividends": {"dividends": [
        {"exDate": "2026-09-15T00:00:00Z", "payableDate": "2026-09-30", "amount": "0.05", "currency": "CAD"},
        {"exDate": "2026-06-15", "payableDate": None, "amount": 0.05, "currency": "CAD"},
        {"exDate": "bad", "amount": 1},
        {"exDate": "2026-03-15", "amount": 0},
        {"exDate": "2026-01-15", "amount": None},
        "not a dict",
    ]}}},
    {"data": {"dividends": {}}},
    {},
]

VENUES = ["Toronto Stock Exchange", "TSX Venture Exchange", "Canadian Securities Exchange",
          "Cboe Canada", "NEO Exchange", "Nasdaq", "New York Stock Exchange", "NYSE American",
          "Somewhere Else", ""]

RECORDS = [["QNC", "TSX"], ["QNC.TO", "TSX"], ["QNC", "CSE"], ["QNC", "Cboe Canada"],
           ["QNC", "NASDAQ"], ["QNC", ""], ["", "TSX"]]
BARES = ["QNC", "QNC:CNX", "QNC:US", "", "^TSX"]
CANADIAN = [["TSX", "CAD"], ["NASDAQ", "USD"], ["", "CAD"], ["", "USD"], ["ALPHA EXCHANGE", ""], ["", ""]]


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
    payload = {"quotes": QUOTES, "dividends": DIVIDENDS, "venues": VENUES,
               "records": RECORDS, "bares": BARES, "canadian": CANADIAN}
    got = json.loads(subprocess.run([BIN, "pure"], input=json.dumps(payload),
                                    capture_output=True, text=True, check=True).stdout)
    bad = []
    for i, q in enumerate(QUOTES):
        if norm(market.parse_tmx_quote(q)) != norm(got["quotes"][i]):
            bad.append(f"quote[{i}]: py={market.parse_tmx_quote(q)} rs={got['quotes'][i]}")
    for i, d in enumerate(DIVIDENDS):
        if norm(market.parse_tmx_dividends(d)) != norm(got["dividends"][i]):
            bad.append(f"dividends[{i}]: py={market.parse_tmx_dividends(d)} rs={got['dividends'][i]}")
    for i, v in enumerate(VENUES):
        if market.tmx_venue(v) != got["venues"][i]:
            bad.append(f"venue[{i}] {v!r}: py={market.tmx_venue(v)!r} rs={got['venues'][i]!r}")
    for i, (s, e) in enumerate(RECORDS):
        if market.tmx_record_symbol(s, e) != got["records"][i]:
            bad.append(f"record[{i}] {s}/{e}: py={market.tmx_record_symbol(s, e)!r} rs={got['records'][i]!r}")
    for i, b in enumerate(BARES):
        if market.tmx_bare(b) != got["bares"][i]:
            bad.append(f"bare[{i}] {b!r}: py={market.tmx_bare(b)!r} rs={got['bares'][i]!r}")
    for i, (e, c) in enumerate(CANADIAN):
        if market.is_canadian_listing(e, c) is not got["canadian"][i]:
            bad.append(f"canadian[{i}] {e}/{c}: py={market.is_canadian_listing(e, c)} rs={got['canadian'][i]}")

    # one live listing, through both, into their own store
    work = tempfile.mkdtemp(prefix="tmxtest-")
    pyhome, rshome = os.path.join(work, "py"), os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)
    import store
    import model
    today = model.today_local()
    for home in (pyhome, rshome):
        store.set_home(home)
        store.ensure()
        store.close_all()

    live = [["QNC", "TSX"], ["SHOP", "TSX"]]
    store.set_home(pyhome)
    want_live = []
    for sym, ex in live:
        q, d = market.fetch_tmx(sym, None, ex)
        want_live.append({"symbol": sym, "quote": q, "dividends": d})
    store.close_all()

    got_live = json.loads(subprocess.run(
        [BIN, "live"],
        input=json.dumps({"db": os.path.join(rshome, "bagholder.db"), "today": today, "symbols": live}),
        capture_output=True, text=True, check=True).stdout)

    for i, (w, g) in enumerate(zip(want_live, got_live)):
        # a price moves between the two calls; the shape and the record are what
        # must match, not the last trade
        for key in ("name", "exchange", "currency", "dividendFrequency", "exDividendDate"):
            a = (w["quote"] or {}).get(key)
            b = (g["quote"] or {}).get(key)
            if a != b:
                bad.append(f"live[{i}].quote.{key}: py={a!r} rs={b!r}")
        if norm(w["dividends"]) != norm(g["dividends"]):
            bad.append(f"live[{i}].dividends: py={w['dividends']} rs={g['dividends']}")
    store.close_all()
    shutil.rmtree(work, ignore_errors=True)

    for line in bad[:20]:
        print("  " + line)
    n = len(QUOTES) + len(DIVIDENDS) + len(VENUES) + len(RECORDS) + len(BARES) + len(CANADIAN) + len(live)
    print(f"{n} TMX cases ({len(live)} live), {len(bad)} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
