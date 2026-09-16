"""Every market parser, both ways, on the shapes the feeds actually send and
on the shapes they send when something is wrong: truncated text, a holiday
marked with a dot, a null price, a row that is not an object, a quote taken
outside a session where `last` is zero.

    cargo build -p bagholder-market && python3 crates/market/parsetest.py
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
import market  # noqa: E402

BIN = os.path.join(ROOT, "target", "debug", "parsetool")

BOC = json.dumps({"observations": [
    {"d": "2024-01-02", "FXUSDCAD": {"v": "1.3316"}},
    {"d": "2024-01-03", "FXUSDCAD": {"v": "1.3400"}},
    {"d": "2024-01-04", "FXUSDCAD": {"v": "0"}},
    {"d": "2024-01-05", "FXUSDCAD": {}},
    {"d": "bad", "FXUSDCAD": {"v": "1.30"}},
    {"d": "2024-01-06"},
    "not a dict",
]})

FRED = "observation_date,SP500\n2024-01-02,4742.83\n2024-01-03,.\n2024-01-04,\n2024-01-05,4697.24\nbad,1\n2024-01-06,0\n"
STOOQ = "Date,Open,High,Low,Close,Volume\n2024-01-02,1,2,0.5,4742.83,100\n2024-01-03,1,2,0.5,x,100\nshort,1\n2024-01-04,1,2,0.5,0,100\n"

TMX_HISTORY = {"data": {"getTimeSeriesData": [
    {"dateTime": "2024-01-03T00:00:00Z", "open": 1, "high": 2, "low": 0.5, "close": 1.5, "volume": 10},
    {"dateTime": "2024-01-02", "open": None, "high": None, "low": None, "close": 1.0, "volume": None},
    {"dateTime": "bad"},
    "not a dict",
]}}

CBOE_CA_HISTORY = json.dumps({"data": [
    {"date": "2024-01-03", "open": 1, "high": 2, "low": 0.5, "close": 1.5, "volume": 10},
    {"date": "2024-01-02", "close": 1.0},
    {"date": ""},
]})

CBOE_CA_QUOTE_LIVE = json.dumps({"data": {"last": 12.5, "prev_close": 12.0, "change": 0.5,
                                          "change_pct": 4.1667, "company_name": "Thing Inc"}})
CBOE_CA_QUOTE_CLOSED = json.dumps({"data": {"last": 0, "prev_close": 12.0, "change": None,
                                            "change_pct": None, "company_name": ""}})
CBOE_CA_QUOTE_EMPTY = json.dumps({"data": {"last": 0, "prev_close": 0}})

CBOE_OPTIONS = json.dumps({"data": {"options": [
    {"option": "QNC261120C00003000", "bid": 1.0, "ask": 1.2, "prev_day_close": 1.05, "last_trade_price": 1.1},
    {"option": "QNC261120P00003000", "bid": 0, "ask": 0, "prev_day_close": 2.0, "last_trade_price": 0},
    "not a dict",
]}})

MARKS = [
    {"bid": 1.0, "ask": 1.2, "prev_day_close": 1.05, "last_trade_price": 1.1},
    {"bid": 0, "ask": 0, "prev_day_close": 2.0, "last_trade_price": 0},
    {"bid": 0, "ask": 0, "prev_day_close": None, "last_trade_price": 3.0},
    {"bid": 0, "ask": 0, "prev_day_close": 0, "last_trade_price": 0},
    {"bid": 1.0, "ask": 0, "prev_day_close": None, "last_trade_price": 0},
]

COINBASE = json.dumps({"data": {"amount": "63150.25", "currency": "CAD"}})
COINBASE_NO_CCY = json.dumps({"data": {"amount": "63150.25"}})
COINBASE_ZERO = json.dumps({"data": {"amount": "0"}})
CANDLES = json.dumps([
    [1700000000, 1.0, 2.0, 1.5, 1.8, 100.0],
    [1699999000, 1.0, 2.0, 1.5, 1.7, 50.0],
    [1699998000, 1.0, 2.0, 1.5, 0, 50.0],
    [1699997000, 1.0],
    "not a list",
])

OCC = ["QNC 20NOV26 3.00 CALL", "QNC 20NOV26 3.00 PUT", "QNC 20NOV26 3 C", "SPY 261120C00450000",
       "QNC  20NOV26  3.00  CALL", "QNC 20XXX26 3.00 CALL", "AAA", "", "TOOLONGSYMBOL 20NOV26 3.00 CALL",
       "QNC 5NOV26 12.5 PUT", "qnc 20nov26 3.00 call"]
YAHOO = ["YES.V", "SHOP.TO", "ABC.CN", "DEF.NE", "AAPL", ".TO", "", "shop.to"]


def cases():
    out = [
        {"fn": "boc", "arg": BOC}, {"fn": "boc", "arg": "{"}, {"fn": "boc", "arg": ""},
        {"fn": "fred", "arg": FRED}, {"fn": "fred", "arg": ""},
        {"fn": "stooq", "arg": STOOQ}, {"fn": "stooq", "arg": ""},
        {"fn": "tmx_history", "arg": TMX_HISTORY}, {"fn": "tmx_history", "arg": {}},
        {"fn": "cboe_ca_history", "arg": CBOE_CA_HISTORY}, {"fn": "cboe_ca_history", "arg": "{}"},
        {"fn": "cboe_ca_quote", "arg": CBOE_CA_QUOTE_LIVE},
        {"fn": "cboe_ca_quote", "arg": CBOE_CA_QUOTE_CLOSED},
        {"fn": "cboe_ca_quote", "arg": CBOE_CA_QUOTE_EMPTY},
        {"fn": "cboe_options", "arg": CBOE_OPTIONS}, {"fn": "cboe_options", "arg": "{}"},
        {"fn": "coinbase", "arg": COINBASE, "pair": "BTC-CAD"},
        {"fn": "coinbase", "arg": COINBASE_NO_CCY, "pair": "BTC-CAD"},
        {"fn": "coinbase", "arg": COINBASE_ZERO, "pair": "BTC-CAD"},
        {"fn": "coinbase_candles", "arg": CANDLES}, {"fn": "coinbase_candles", "arg": "[]"},
    ]
    out += [{"fn": "option_mark", "arg": m} for m in MARKS]
    out += [{"fn": "occ", "arg": s} for s in OCC]
    out += [{"fn": "yahoo_split", "arg": s} for s in YAHOO]
    return out


def python_side(c):
    f, arg = c["fn"], c["arg"]
    if f == "boc":
        return market.parse_boc_json(arg)
    if f == "fred":
        return market.parse_fred_csv(arg)
    if f == "stooq":
        return market.parse_stooq_csv(arg)
    if f == "tmx_history":
        return market.parse_tmx_history(arg)
    if f == "cboe_ca_history":
        return market.parse_cboe_ca_history(arg)
    if f == "cboe_ca_quote":
        return market.parse_cboe_ca_quote(arg)
    if f == "cboe_options":
        return market.parse_cboe_options(arg)
    if f == "option_mark":
        return market.option_mark(arg)
    if f == "coinbase":
        return market.parse_coinbase(arg, c.get("pair", ""))
    if f == "coinbase_candles":
        return market.parse_coinbase_candles(arg)
    if f == "occ":
        return market.occ_code(arg)
    if f == "yahoo_split":
        t, v = market.yahoo_split(arg)
        return [t, list(v) if v else None]
    raise SystemExit("unknown " + f)


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
    cs = cases()
    got = json.loads(subprocess.run([BIN], input=json.dumps(cs), capture_output=True,
                                    text=True, check=True).stdout)
    bad = 0
    for c, g in zip(cs, got):
        w = python_side(c)
        if norm(w) != norm(g):
            bad += 1
            arg = c["arg"] if isinstance(c["arg"], str) else json.dumps(c["arg"])
            print(f"  {c['fn']}({arg[:40]!r})")
            print(f"    py={json.dumps(norm(w))[:180]}")
            print(f"    rs={json.dumps(norm(g))[:180]}")
    print(f"{len(cs)} parser cases, {bad} differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
