"""Differential test for shorts.py against crates/market/src/shorts.rs.

Both are handed the regulators' real files -- FINRA's daily volume files and
settlement answers, CIRO's position reports and short sale summaries -- plus
hand-made edges, and every parsed row is compared. The record for one listing
is compared on fixed answers (Python's own `for_listing` with its fetches
replaced by those answers, against the Rust assembly), over a copy of the
database so days to cover reads the same index calendar. With `--live` both
also read the real regulators for a handful of listings end to end.
"""
import json
import math
import os
import random
import shutil
import subprocess
import sys
from datetime import date, datetime, timedelta, timezone

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")))
import market  # noqa: E402
import shorts  # noqa: E402
import store  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "..", "target", "release", "shortstool")
WORK = os.environ.get("SHORTSTEST_DIR") or "/tmp/shortstest"
XLS = os.environ.get("XLSTEST_DIR") or "/tmp/xlstest"


def cached(name, fetch):
    path = os.path.join(WORK, name)
    if os.path.exists(path):
        return open(path, "rb").read()
    raw = fetch()
    open(path, "wb").write(raw)
    return raw


def run_rust(payload, mode=""):
    p = subprocess.run([TOOL] + ([mode] if mode else []), input=json.dumps(payload), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("shortstool failed: %s" % p.stderr[-2000:])
    return json.loads(p.stdout)


def same(a, b):
    if isinstance(a, float) and isinstance(b, (int, float)):
        return (math.isnan(a) and math.isnan(b)) or a == b
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    return a == b


def jsonable(v):
    """What json.dumps would write, with NaN and infinities as the null serde writes."""
    if isinstance(v, float) and not math.isfinite(v):
        return None
    if isinstance(v, dict):
        return {k: jsonable(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [jsonable(x) for x in v]
    return v


def diff(label, want, got, bad):
    if not same(want, got):
        bad.append("%s\n  python: %s\n  rust:   %s" % (label, json.dumps(want, default=str)[:600], json.dumps(got, default=str)[:600]))


def main():
    os.makedirs(WORK, exist_ok=True)
    db_src = os.environ.get("BAGHOLDER_DB") or os.path.expanduser("~/.bagholder/bagholder.db")
    home = os.path.join(WORK, "home")
    os.makedirs(home, exist_ok=True)
    db = os.path.join(home, "bagholder.db")
    shutil.copy(db_src, db)
    store.set_home(home)
    ctx = market.default_ssl_context()
    rnd = random.Random(20260916)
    today = datetime.now(timezone.utc).date()
    bad, n = [], 0

    # --- dates
    days = [date(2000, 1, 1) + timedelta(days=rnd.randrange(365 * 31)) for _ in range(400)]
    days += [date(2024, 2, 29), date(2023, 2, 28), date(2026, 12, 31), date(2027, 1, 1), date(2026, 1, 15),
             date(2026, 1, 14), date(2026, 1, 16), date(2026, 3, 1), date(2026, 9, 13), date(2026, 9, 14)]
    # --- numbers
    floats = ["1", " 2.5 ", "1,234.5", "-0", "+3", "1e3", "1E-3", "inf", "-Infinity", "nan", "NaN", "1_000",
              "_1", "1__0", "1_", "", " ", "abc", "1.2.3", "0x10", "1e", ".5", "5.", "--1", "+-1", "١٢", "infinit",
              "Infinity", "  -iNf ", "1d", "1f", 7, 7.25, True, None, "１２"]
    # --- the regulators' own files
    us_texts, ca_texts = [], []
    for d in shorts.trading_days(today, back=4):
        name = "finra-%s.txt" % d.strftime("%Y%m%d")
        try:
            us_texts.append(cached(name, lambda: market._fetch(shorts.US_VOLUME_URL % d.strftime("%Y%m%d"), ctx, shorts.HEADERS, market.TIMEOUT_SEC)).decode("utf-8", "replace"))
        except Exception:
            continue
    for start, end in shorts.volume_periods(today, back=3):
        name = "ciro-%s-%s.csv" % (start.strftime("%Y%m%d"), end.strftime("%Y%m%d"))
        try:
            ca_texts.append(cached(name, lambda: market._fetch(shorts.CA_VOLUME_URL % (start.strftime("%Y%m%d"), end.strftime("%Y%m%d")), ctx, shorts.HEADERS, market.TIMEOUT_SEC)).decode("utf-8", "replace"))
        except Exception:
            continue
    print("  files: %d FINRA daily, %d CIRO summaries" % (len(us_texts), len(ca_texts)))
    us_texts += ["", "header only", "Date|Symbol|ShortVolume|ShortExemptVolume|TotalVolume|Market\n20260101|aapl|1,000|0|2000|Q\n20260101||5|0|10|Q\n"
                 "20260101|ZERO|5|0|0|Q\n20260101|NANT|5|0|nan|Q\n20260101|SHORT|x|0|10|Q\n20260101|FEW|1|2\r\n20260101| spaced |1|0|3|Q\x0b20260101|VT|1|0|3|Q x|U28|1|0|3|Q"]
    ca_texts += ["", "﻿Security,Listing Market,Short Traded Volume,% Total Traded Volume\n"
                 "abc,tsx,\"1,000\",10\n\"Q,UOTED\",CSE,5,0\n\"D\"\"Q\",NEO,5,\n\nSHORTROW,TSX\nEXTRA,TSX,1,50,more,fields\n"
                 "NOSHORT,TSX,,3\n\"multi\nline\",TSX,2,4\r\nCR,TSX,1,1\rLAST,TSX,9,3"]
    csv_head = "Security,Listing Market,Short Traded Volume,% Total Traded Volume\n"
    ca_texts += [csv_head + body for body in [
        '"abc', '"abc"x"y",TSX,1,2', '""\nA,TSX,1,2', 'x\r', 'x\ry,TSX,1,1', '"q\r\nq",TSX,1,1\n', 'Q,TSX,1,""""',
        'ab"c,TSX,1,1', '  \nB,TSX,1,2', '"x"\r1', 'C,TSX,1,2\r\n\r\nD,TSX,3,4', 'E,TSX,1,2\r', ',,,', 'F,"TSX",  7 ,"1,5"']]
    ca_texts += ["", "\n\n", "Security\n", "\ufeff\ufeffSecurity,Short Traded Volume\nX,1"]
    # a real summary with a stray carriage return put in a row, which Python refuses
    if ca_texts and "\n" in ca_texts[0][100:]:
        k = ca_texts[0].index("\n", 100)
        ca_texts.append(ca_texts[0][:k - 3] + "\r" + ca_texts[0][k - 3:])
    xls_files = [os.path.join(XLS, f) for f in sorted(os.listdir(XLS)) if f[:1].isdigit()] if os.path.isdir(XLS) else []
    # --- FINRA settlements
    finra, finra_syms = [], []
    for sym in ["AAPL", "GME", "TSLA", "AMC", "SPY", "BRK.B", "NOPE"]:
        name = "finra-pos-%s.json" % sym
        body = {"limit": 20, "compareFilters": [{"fieldName": "symbolCode", "fieldValue": sym, "compareType": "EQUAL"}],
                "dateRangeFilters": [{"fieldName": "settlementDate", "startDate": (today - timedelta(days=150)).isoformat(), "endDate": today.isoformat()}]}
        try:
            finra.append(json.loads(cached(name, lambda: json.dumps(market._post_json(shorts.US_POSITION_URL, body, ctx, {"Accept": "application/json"})).encode())))
            finra_syms.append(sym)
        except Exception as e:
            print("  finra %s: %s" % (sym, e))
    print("  FINRA settlement answers: %d" % len(finra))
    finra += [[], None, {"not": "a list"}, [{"settlementDate": ""}, "x", {"settlementDate": "2026-01-15T00:00", "currentShortPositionQuantity": "12",
              "previousShortPositionQuantity": None, "changePreviousNumber": "abc", "averageDailyVolumeQuantity": 0},
              {"settlementDate": "2026-01-15", "currentShortPositionQuantity": 5}, {"settlementDate": "2025-12-31", "currentShortPositionQuantity": None}]]

    fits = [[c, e] for c in ["TSX", "TSXV", "CSE", "AQL", "tsx", " AQL ", "", "X"] for e in ["", "TSX", "TSX-V", "TSXV", "CSE", "Cboe Canada", "NEO", " tsx ", "NYSE"]]
    markets = [["AAPL", "NASDAQ", "USD"], ["SHOP", "TSX", "CAD"], ["BTC", "CRYPTO", "CAD"], ["AAPL 260117C00100000", "NASDAQ", "USD"],
               ["SPX", "INDEX", ""], ["GC", "COMEX", "USD"], ["X", "", ""], ["X", "", "USD"], ["X", "", "CAD"], ["", "TSX", "CAD"],
               ["ZEB", "Cboe Canada", "CAD"], ["ABC", "CSE", "CAD"], ["ABC", "LSE", "GBP"], ["abc", " tsx ", "cad"]]
    funds = ["iShares Core S&P 500 Index ETF", "Vanguard FTSE Canada", "Royal Bank of Canada", "ETFs are fun", "The Fund",
             "Trustmark", "BMO Equal Weight Banks", "BMOX", "Global X Uranium", "global xyz", "Horizons", "Harvest Portfolios",
             "Portfolio_ Co", "fund-of-funds", "NINEPOINT", "  evolve  ", "", "Évolve", "Real Estate Investment TRUST", "índex"]

    # --- one listing from fixed answers
    pos_rows, vol_rows = {}, {}
    if xls_files:
        import xls
        pos_rows = shorts.parse_ca_positions(xls.table(open(xls_files[-1], "rb").read()))
    if ca_texts:
        vol_rows = shorts.parse_ca_volume(ca_texts[0])
    us_rows = shorts.parse_us_volume(us_texts[0]) if us_texts else {}
    finish = []
    pos_key = os.path.basename(xls_files[-1])[:8] if xls_files else ""
    pos_key = "%s-%s-%s" % (pos_key[:4], pos_key[4:6], pos_key[6:8]) if pos_key else ""
    vol_key = "2026-08-16/2026-08-31"
    ca_syms = sorted(pos_rows)[:0] + rnd.sample(sorted(pos_rows), min(40, len(pos_rows))) if pos_rows else []
    ca_syms += [s for s in sorted(vol_rows)[:10]] + ["NOTANYWHERE"]
    for i, sym in enumerate(ca_syms):
        row = pos_rows.get(sym) or {}
        venue_names = {"TSX": "TSX", "TSXV": "TSX-V", "CSE": "CSE", "AQL": "Cboe Canada"}
        ex = rnd.choice(["", venue_names.get(row.get("venue", ""), "TSX"), "CSE"])
        finish.append({"today": today.isoformat(), "symbol": sym, "exchange": ex, "currency": "CAD", "name": rnd.choice(["", "Some Fund"]),
                       "trend": bool(i % 2), "series": [{"date": "2026-08-15", "shares": 10.0}] if i % 3 else [],
                       "float": rnd.choice([None, 0.0, 1234567.0]), "traded": rnd.choice([None, 0.0, 55555.0]),
                       "files": {"ca_position": {"key": pos_key, "rows": pos_rows}, "ca_volume": {"key": rnd.choice([vol_key, "", "2026-09-01/2026-09-15"]), "rows": vol_rows}}})
    for sym, ans in zip(finra_syms, finra):
        finish.append({"today": today.isoformat(), "symbol": sym, "exchange": "NASDAQ", "currency": "USD", "name": "", "trend": True,
                       "series": [], "float": rnd.choice([None, 15e9]), "finra": ans,
                       "files": {"us_volume": {"key": "2026-09-15", "rows": us_rows}}})
    finish.append({"today": today.isoformat(), "symbol": "BTC", "exchange": "CRYPTO", "currency": "CAD", "files": {}})

    payload = {"db": db, "dates": [d.isoformat() for d in days], "floats": floats, "us_volume": us_texts, "ca_volume": ca_texts,
               "ca_positions": xls_files, "us_position": finra, "fits": fits, "markets": markets, "funds": funds, "finish": finish}
    got = run_rust(payload)

    for i, d in enumerate(days):
        g = got["dates"][i]
        diff("position_dates %s" % d, [x.isoformat() for x in shorts.position_dates(d)], g["positions"], bad)
        diff("position_dates(8) %s" % d, [x.isoformat() for x in shorts.position_dates(d, back=8)], g["series"], bad)
        diff("volume_periods %s" % d, [[a.isoformat(), b.isoformat()] for a, b in shorts.volume_periods(d)], g["periods"], bad)
        diff("trading_days %s" % d, [x.isoformat() for x in shorts.trading_days(d)], g["trading"], bad)
        n += 4
    for i, v in enumerate(floats):
        want = shorts._num(v)
        g = got["floats"][i]
        g = None if g is None else float(g)
        diff("_num(%r)" % (v,), want, g, bad)
        n += 1
    for i, t in enumerate(us_texts):
        want = jsonable(shorts.parse_us_volume(t))
        diff("parse_us_volume[%d]" % i, want, got["us_volume"][i], bad)
        n += max(1, len(want))
    for i, t in enumerate(ca_texts):
        try:
            want = jsonable(shorts.parse_ca_volume(t))
        except Exception:
            want = "raised"
        diff("parse_ca_volume[%d]" % i, want, got["ca_volume"][i], bad)
        n += max(1, len(want))
    if xls_files:
        import xls
        for i, f in enumerate(xls_files):
            want = jsonable(shorts.parse_ca_positions(xls.table(open(f, "rb").read())))
            diff("parse_ca_positions %s" % os.path.basename(f), want, got["ca_positions"][i], bad)
            n += len(want)

    # FINRA's answer through Python's own us_position, its request replaced
    for i, ans in enumerate(finra):
        real = market._post_json
        market._post_json = lambda *a, _ans=ans, **k: _ans
        try:
            try:
                want = shorts.us_position("X", None, datetime.now(timezone.utc))
            except Exception as e:
                want = "raised %s" % type(e).__name__
        finally:
            market._post_json = real
        diff("us_position[%d]" % i, jsonable(want), got["us_position"][i], bad)
        n += 1
    for i, (c, e) in enumerate(fits):
        diff("_venue_fits(%r, %r)" % (c, e), shorts._venue_fits(c, e), got["fits"][i], bad)
        n += 1
    for i, m in enumerate(markets):
        diff("market_of%r" % (m,), shorts.market_of(*m), got["markets"][i], bad)
        n += 1
    import exposure
    for i, name in enumerate(funds):
        diff("is_fund(%r)" % name, exposure.is_fund(name), got["funds"][i], bad)
        n += 1

    # the record, through Python's for_listing with every fetch replaced
    saved = {k: getattr(shorts, k) for k in ("_table", "ca_traded", "float_shares", "ca_series")}
    real_post = market._post_json
    try:
        for i, c in enumerate(finish):
            files = c.get("files") or {}
            shorts._table = lambda name, build, ctx, now, _f=files: dict(_f.get(name) or {"key": "", "rows": {}}, at=0)
            shorts.ca_traded = lambda *a, _t=c.get("traded"), **k: _t
            shorts.float_shares = lambda *a, _f=c.get("float"), **k: _f
            shorts.ca_series = lambda *a, _s=c.get("series"), **k: _s
            market._post_json = lambda *a, _ans=c.get("finra"), **k: _ans
            now = datetime.fromisoformat(c["today"]).replace(tzinfo=timezone.utc)
            want = shorts.for_listing(c["symbol"], c.get("exchange", ""), c.get("currency", ""), None, now, c.get("trend", False), c.get("name", ""))
            diff("for_listing[%d] %s %s" % (i, c["symbol"], c.get("exchange")), jsonable(want), got["finish"][i], bad)
            n += 1
    finally:
        for k, v in saved.items():
            setattr(shorts, k, v)
        market._post_json = real_post

    if "--live" in sys.argv:
        listings = [["AAPL", "NASDAQ", "USD", False, ""], ["GME", "NYSE", "USD", False, ""], ["SPY", "NYSE", "USD", False, "SPDR S&P 500 ETF Trust"],
                    ["RY", "TSX", "CAD", True, ""], ["SHOP", "TSX", "CAD", False, ""], ["XIU", "TSX", "CAD", False, "iShares S&P/TSX 60 Index ETF"],
                    ["BTC", "CRYPTO", "CAD", False, ""]]
        rust = run_rust({"db": db, "today": today.isoformat(), "listings": listings}, "live")
        for i, l in enumerate(listings):
            want = jsonable(shorts.for_listing(l[0], l[1], l[2], None, None, l[3], l[4]))
            diff("live %s" % l[:2], want, rust[i], bad)
            n += 1
            print("  live %-6s %-6s float %s / %s  days to cover %s / %s" % (l[0], l[1], want.get("float"), rust[i].get("float"),
                                                                            want.get("daysToCover"), rust[i].get("daysToCover")))

    print("%d comparisons, %d differing" % (n, len(bad)))
    for b in bad[:25]:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
