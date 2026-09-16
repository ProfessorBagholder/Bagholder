"""Differential test for universes.py against crates/market/src/universes.rs.

The two parsers are handed the same screener and index answers -- the shapes
Nasdaq and TMX send, the page-written numbers (`$1.23`, `-0.45%`, `1,234`,
`N/A`) and the rows each must refuse -- and the tiles are compared field by
field, in order. With `--live` both read the real screener and the real
S&P/TSX 60 and the two answers are compared by symbol.
"""
import json
import os
import random
import subprocess
import sys

sys.path.insert(0, "/Users/md/dev/Bagholder")
import universes  # noqa: E402

TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "target", "release", "unitool")

SECTORS = ["Technology", "technology ", "Finance", "Basic Materials", "Health Care", "Consumer Discretionary",
           "Energy", "Real Estate", "Telecommunications", "Utilities", "Industrials", "Consumer Staples",
           "Miscellaneous", "", "  ", "Not classified", "nonsense sector"]

SCREENER = [
    {"data": {"rows": [
        {"symbol": "AAPL", "name": "Apple Inc. Common Stock", "lastsale": "$245.50", "pctchange": "1.23%",
         "marketCap": "3644000000000.00", "sector": "Technology", "country": "United States"},
        {"symbol": " MSFT ", "name": " Microsoft ", "lastsale": "$400.00", "pctchange": "-0.45%",
         "marketCap": "2,900,000,000,000", "sector": "technology", "country": "United States"},
        {"symbol": "TSM", "name": "Taiwan Semi", "lastsale": "$180.00", "pctchange": "0.00%",
         "marketCap": "900000000000", "sector": "Technology", "country": "Taiwan"},
        {"symbol": "SHOP", "name": "Shopify", "lastsale": "$100.00", "pctchange": "2%",
         "marketCap": "130000000000", "sector": "Technology", "country": "Canada"},
        {"symbol": "NOWHERE", "name": "No country", "lastsale": "$1.00", "pctchange": "1%",
         "marketCap": "500000000", "sector": "Energy", "country": ""},
        # refused and defaulted
        {"name": "no symbol", "marketCap": "1", "country": "United States"},
        {"symbol": "NOCAP", "name": "No cap", "lastsale": "N/A", "pctchange": "N/A",
         "marketCap": "", "sector": "", "country": "United States"},
        {"symbol": "ZEROCAP", "name": "Zero cap", "lastsale": "NA", "pctchange": "None",
         "marketCap": "0.00", "sector": None, "country": "United States"},
        {"symbol": "NEGZERO", "name": "Negative zero", "lastsale": "$-0.00", "pctchange": "-0.00",
         "marketCap": "-0.0", "sector": "Energy", "country": "United States"},
        {"symbol": "JUNK", "name": "Junk numbers", "lastsale": "abc", "pctchange": "--",
         "marketCap": "1e11", "sector": "Health Care", "country": "Ireland"},
        {"symbol": "NULLS", "name": None, "lastsale": None, "pctchange": None,
         "marketCap": None, "sector": "Finance", "country": None},
        "not a dict",
        {"symbol": "TIE1", "name": "Tie one", "marketCap": "1000", "pctchange": "1%", "country": "United States"},
        {"symbol": "TIE2", "name": "Tie two", "marketCap": "1000", "pctchange": "2%", "country": "United States"},
        {"symbol": "TIE3", "name": "Tie three", "marketCap": "1000", "pctchange": "3%", "country": "Bermuda"},
        {"symbol": "TIE4", "name": "Tie four", "marketCap": "1000", "pctchange": "4%", "country": "Bermuda"},
    ]}},
    {"data": {"rows": []}},
    {"data": {}},
    {},
    None,
]

CONSTITUENTS = [
    {"data": {"constituents": [
        {"symbol": "RY", "longName": "Royal Bank of Canada", "shortName": "Royal Bank",
         "weight": "7.23", "quotedMarketValue": "250000000000", "exchange": "TSX"},
        {"symbol": " TD ", "longName": "", "shortName": "TD Bank", "weight": "5.1",
         "quotedMarketValue": "160000000000", "exchange": " TSX "},
        {"symbol": "NOWEIGHT", "longName": None, "shortName": None, "weight": None,
         "quotedMarketValue": "1000", "exchange": None},
        {"symbol": "ZEROWEIGHT", "longName": "Zero", "weight": "0", "quotedMarketValue": "0"},
        {"symbol": "NEGZERO", "longName": "Neg zero", "weight": "-0.0", "quotedMarketValue": "-0.0"},
        {"longName": "no symbol", "weight": "1"},
        "not a dict",
    ]}},
    {"data": {"constituents": []}},
    {"data": {}},
    {},
    None,
]

TILES = [
    {"data": {"getQuoteBySymbol": {"symbol": "RY", "name": "Royal Bank of Canada", "price": 180.0,
                                   "percentChange": "0.85", "sector": "Finance"}}},
    {"data": {"getQuoteBySymbol": {"symbol": "X", "name": " Spaced ", "percentChange": None, "sector": None}}},
    {"data": {"getQuoteBySymbol": {}}},
    {"data": {"getQuoteBySymbol": None}},
    {"data": {}},
    {},
    None,
]


def rand_rows(rnd, howmany):
    """A screener answer bigger than the hundred taken, with ties and junk."""
    countries = ["United States", "Canada", "Ireland", "Bermuda", "Taiwan", "Israel", "", None]
    rows = []
    for i in range(howmany):
        cap = rnd.choice(["%d" % rnd.randint(0, 5_000_000_000_000), "N/A", "", "0", "-0.0",
                          "{:,}".format(rnd.randint(1, 10 ** 12)), "1e%d" % rnd.randint(1, 12)])
        rows.append({"symbol": "R%04d" % i, "name": "Rand %d" % i,
                     "lastsale": rnd.choice(["$%0.2f" % rnd.uniform(0, 900), "N/A", ""]),
                     "pctchange": rnd.choice(["%0.2f%%" % rnd.uniform(-9, 9), "N/A", "--"]),
                     "marketCap": cap, "sector": rnd.choice(SECTORS),
                     "country": rnd.choice(countries)})
    return rows


def run_rust(payload, mode=""):
    p = subprocess.run([TOOL] + ([mode] if mode else []), input=json.dumps(payload), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("unitool failed: %s" % p.stderr)
    return json.loads(p.stdout)


def diff(label, want, got, bad):
    if want != got:
        bad.append("%s\n  python: %s\n  rust:   %s" % (label, json.dumps(want)[:400], json.dumps(got)[:400]))


def main():
    rnd = random.Random(20260916)
    parsed = [universes.parse_screener(d) for d in SCREENER]
    big = [rand_rows(rnd, 400) for _ in range(6)]
    rowsets = parsed + [universes.parse_screener({"data": {"rows": r}}) for r in big]

    payload = {"screener": SCREENER, "rows": rowsets, "constituents": CONSTITUENTS,
               "tiles": TILES, "sectors": SECTORS}
    got = run_rust(payload)
    bad, n = [], 0

    for i, d in enumerate(SCREENER):
        diff("parse_screener[%d]" % i, parsed[i], got["screener"][i], bad)
        n += 1
    for i, rows in enumerate(rowsets):
        diff("us_rows[%d]" % i, universes.us_rows(rows), got["us"][i], bad)
        diff("intl_rows[%d]" % i, universes.intl_rows(rows), got["intl"][i], bad)
        n += 2
    for i, d in enumerate(CONSTITUENTS):
        diff("parse_constituents[%d]" % i, universes.parse_constituents(d), got["constituents"][i], bad)
        n += 1
    for i, d in enumerate(TILES):
        diff("parse_tile_quote[%d]" % i, universes.parse_tile_quote(d), got["tiles"][i], bad)
        n += 1
    for i, s in enumerate(SECTORS):
        diff("sector_of[%r]" % s, universes.sector_of(s), got["sectors"][i], bad)
        n += 1

    if "--live" in sys.argv:
        rust = run_rust({}, "live")
        py_screener = universes.fetch_screener()
        pm = {r["symbol"]: r for r in py_screener}
        rm = {r["symbol"]: r for r in (rust["screener"] or [])}
        shared = sorted(set(pm) & set(rm))
        print("  live screener   python %5d  rust %5d  shared %5d" % (len(pm), len(rm), len(shared)))
        for k in shared:
            # the last sale and the day's change move between the two calls;
            # what the row is does not
            for f in ("name", "cap", "sector", "country"):
                diff("live screener %s %s" % (k, f), pm[k][f], rm[k][f], bad)
            n += 4
        # the hundred taken must be the same hundred, in the same order
        diff("live us_rows", [r["symbol"] for r in universes.us_rows(py_screener)],
             [r["symbol"] for r in universes.us_rows(rust["screener"])], bad)
        diff("live intl_rows", [r["symbol"] for r in universes.intl_rows(py_screener)],
             [r["symbol"] for r in universes.intl_rows(rust["screener"])], bad)
        n += 2

        py_ca = universes.fetch_canada()
        pc = {r["symbol"]: r for r in py_ca}
        rc = {r["symbol"]: r for r in (rust["canada"] or [])}
        sc = sorted(set(pc) & set(rc))
        print("  live S&P/TSX 60 python %5d  rust %5d  shared %5d" % (len(pc), len(rc), len(sc)))
        diff("live ca order", [r["symbol"] for r in py_ca], [r["symbol"] for r in (rust["canada"] or [])], bad)
        for k in sc:
            for f in ("name", "value", "sector", "country"):
                diff("live ca %s %s" % (k, f), pc[k][f], rc[k][f], bad)
            n += 4

    print("%d comparisons, %d differing" % (n, len(bad)))
    for b in bad[:20]:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
