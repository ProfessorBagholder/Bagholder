"""Differential test for market.py's intraday chain and the readers ported with
it: minute stamps, TMX's minute feed, session and clock aggregation, reach and
the timeframes offered, OCC roots -- on fixtures and random bars -- and then,
live, both implementations filling their own copy of the database with hourly
and four-hour bars from TMX, Coinbase and Yahoo, the declared distributions,
Coinbase's previous close and the glance quote.
"""
import json
import math
import os
import random
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timedelta, timezone

sys.path.insert(0, "/Users/md/dev/Bagholder")
import market  # noqa: E402
import store  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "..", "target", "release", "intradaytool")


def run_rust(payload, mode=""):
    p = subprocess.run([TOOL] + ([mode] if mode else []), input=json.dumps(payload), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("intradaytool failed: %s" % p.stderr[-3000:])
    return json.loads(p.stdout)


def close(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return a == b or (abs(a - b) <= 1e-9 * max(1.0, abs(a), abs(b)))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(close(a[k], b[k]) for k in a)
    if isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)):
        return len(a) == len(b) and all(close(x, y) for x, y in zip(a, b))
    return a == b


def diff(label, want, got, bad):
    if not close(want, got):
        bad.append("%s\n  python: %s\n  rust:   %s" % (label, json.dumps(want, default=str)[:700], json.dumps(got, default=str)[:700]))


def main():
    rnd = random.Random(20260916)
    now = datetime.now(timezone.utc)
    today = now.date().isoformat()
    bad, n = [], 0

    stamps = ["2026-09-02T09:30:00-04:00", "2026-01-05T16:00:59-05:00", "2026-09-02T09:30:00+05:30", "2026-09-02T09:30:00", "2026-09-02 09:30:00-04:00",
              "2026-09-02T09:30:00Z", "bad", "", "2026-09-02T24:00:00-04:00", "2026-02-30T10:00:00-05:00", "2026-09-02T09:30-04:00", "2026-09-02T09:30:00.123-04:00"]
    tmx = [{"data": {"intraday": [{"dateTime": "2026-09-02T09:31:00-04:00", "open": 1, "high": 2, "low": 0.5, "close": 1.5, "volume": 100},
                                  {"dateTime": "2026-09-02T09:30:00-04:00", "open": None, "high": None, "low": None, "close": "1.2", "volume": None},
                                  {"dateTime": "", "close": 1}, {"dateTime": "x", "close": 1}, {"dateTime": "2026-09-02T09:32:00-04:00", "close": 0},
                                  "junk", {"dateTime": "2026-09-02T09:33:00-04:00", "close": -1}]}}, {}, {"data": {"intraday": None}}]
    sessions, hourly = [], []
    for _ in range(40):
        base = int(datetime(2026, 9, rnd.randint(1, 20), tzinfo=timezone.utc).timestamp())
        bars = []
        for i in range(rnd.randint(0, 300)):
            minute = rnd.randint(0, 1439)
            off = rnd.choice([-14400, -18000, 0])
            c = round(rnd.uniform(1, 100), 4)
            bars.append({"time": base + minute * 60 - off, "day": datetime.fromtimestamp(base, tz=timezone.utc).date().isoformat(), "minute": minute, "offset": off,
                         "open": rnd.choice([None, c]), "high": rnd.choice([None, c + 1]), "low": rnd.choice([None, c - 0.5]), "close": c,
                         "volume": rnd.choice([None, 0, rnd.randint(1, 1000), rnd.uniform(0, 5)])})
        bars.sort(key=lambda b: b["time"])
        sessions.append({"bars": bars, "bucket": rnd.choice([60, 240])})
        hbars = [{"time": b["time"], "open": b["open"] if b["open"] is not None else b["close"], "high": b["high"] if b["high"] is not None else b["close"],
                  "low": b["low"] if b["low"] is not None else b["close"], "close": b["close"], "volume": b["volume"]} for b in bars]
        hourly.append({"bars": hbars, "seconds": rnd.choice([3600, 14400])})
    recs = [{"symbol": "SHOP", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "start": today},
            {"symbol": "AAPL", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares", "start": "2024-01-01"},
            {"symbol": "BTC", "currency": "CAD", "kind": "Crypto", "start": "2016-01-01"},
            {"symbol": "BTC", "currency": "CAD", "kind": "Crypto", "start": "2010-01-01"},
            {"symbol": "X", "exchange": "LSE", "currency": "GBP", "kind": "Shares", "start": today},
            {"symbol": "QNC 20NOV26 3.00 CALL", "currency": "USD", "kind": "Options", "start": today},
            {"symbol": "RY", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "start": (now.date() - timedelta(days=366)).isoformat()}]
    occ = ["QNC261120C00003000", "X2261120P00001500", "BRK.B261120C00100000", "261120C00003000", "abc261120C00003000", "QNC261120X00003000", ""]
    floats = [1.0, 0.1, 1e-5, 12345.678, 1e16, 1.5e-7, 60000.123456789, 2.0 ** 60]

    payload = {"today": today, "stamps": stamps, "tmx": tmx, "sessions": sessions, "hourly": hourly, "recs": recs, "occ": occ, "floats": floats}
    got = run_rust(payload)
    for i, s in enumerate(stamps):
        try:
            want = list(market._minute_stamp(s))
        except (ValueError, TypeError):
            want = None
        diff("_minute_stamp(%r)" % s, want, got["stamps"][i], bad)
    for i, d in enumerate(tmx):
        diff("parse_tmx_minutes[%d]" % i, market.parse_tmx_minutes(d), got["tmx"][i], bad)
    for i, c in enumerate(sessions):
        diff("aggregate_session[%d]" % i, market.aggregate_session(c["bars"], c["bucket"]), got["sessions"][i], bad)
    for i, c in enumerate(hourly):
        diff("aggregate_hourly[%d]" % i, market.aggregate_hourly(c["bars"], c["seconds"]), got["hourly"][i], bad)
    for i, r in enumerate(recs):
        diff("reach/available %s" % r["symbol"], {"reach": market.intraday_reach(r, now), "available": market.available_timeframes(r, r["start"], now)}, got["recs"][i], bad)
    for i, c in enumerate(occ):
        diff("occ_root(%r)" % c, market.occ_root(c), got["occ"][i], bad)
    for i, f in enumerate(floats):
        diff("repr(%r)" % f, repr(f), got["floats"][i], bad)
    n += len(stamps) + len(tmx) + len(sessions) + len(hourly) + len(recs) + len(occ) + len(floats)

    if "--live" in sys.argv:
        live = os.environ.get("BAGHOLDER_DB") or os.path.expanduser("~/.bagholder/bagholder.db")
        work = tempfile.mkdtemp(prefix="intradaytest-")
        try:
            py_home, rs_home = os.path.join(work, "py"), os.path.join(work, "rs")
            os.makedirs(py_home)
            os.makedirs(rs_home)
            shutil.copy(live, os.path.join(py_home, "bagholder.db"))
            shutil.copy(live, os.path.join(rs_home, "bagholder.db"))
            store.set_home(py_home)
            start = (now.date() - timedelta(days=12)).isoformat()
            cases = [{"rec": {"symbol": "SHOP", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}, "tf": "1h", "start": start, "end": today},
                     {"rec": {"symbol": "SHOP", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}, "tf": "4h", "start": start, "end": today},
                     {"rec": {"symbol": "BTC", "currency": "CAD", "kind": "Crypto"}, "tf": "4h", "start": start, "end": today},
                     {"rec": {"symbol": "AAPL", "exchange": "NASDAQ", "currency": "USD", "kind": "Shares"}, "tf": "1h", "start": start, "end": today},
                     {"rec": {"symbol": "ZZZZQ", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}, "tf": "1h", "start": start, "end": today},
                     {"rec": {"symbol": "SHOP", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}, "tf": "1d", "start": start, "end": today}]
            payers = [{"symbol": "ZEB", "exchange": "TSX", "currency": "CAD"}, {"symbol": "ENB", "exchange": "TSX", "currency": "CAD"}]
            pairs = ["BTC-CAD", "ETH-USD", "NOPE-CAD"]
            peeks = [{"symbol": "RY", "exchange": "TSX", "currency": "CAD", "kind": "Shares"}, {"symbol": "BTC", "currency": "CAD", "kind": "Crypto"}, {"symbol": "X", "exchange": "LSE", "currency": "GBP"}]
            archive = [{"symbol": "ENB", "exchange": "TSX", "currency": "CAD", "kind": "Shares", "start": start}]
            py = {"intraday": [], "prev": [], "peeks": []}
            for c in cases:
                bars = market.ensure_bars(c["rec"], c["tf"], c["start"], c["end"])
                py["intraday"].append({"bars": bars, "offered": market.offered_timeframes(c["rec"], c["start"]),
                                       "ready": market.intraday_ready(c["rec"], c["tf"], c["start"]), "reason": market.chart_reason(c["rec"], c["tf"])})
            py["distributions"] = market.refresh_distributions(payers, force=True)
            py["prev"] = [market.coinbase_prev_close(p) for p in pairs]
            py["peeks"] = [market.peek_quote(r) for r in peeks]
            py["archived"] = market.archive_intraday(archive)
            rs = run_rust({"db": os.path.join(rs_home, "bagholder.db"), "today": today, "intraday": cases, "payers": payers, "pairs": pairs, "peeks": peeks, "archive": archive}, "live")
            cutoff = int((now - timedelta(hours=6)).timestamp())
            for i, c in enumerate(cases):
                a, b = py["intraday"][i], rs["intraday"][i]
                # the newest hours move between the two reads; everything older must agree
                key = "time" if c["tf"] in ("1h", "4h") else "date"
                older = lambda bars: [x for x in bars if (x[key] < cutoff if key == "time" else x[key] < (now.date() - timedelta(days=1)).isoformat())]
                diff("live bars %s %s" % (c["rec"]["symbol"], c["tf"]), older(a["bars"]), older(b["bars"]), bad)
                diff("live offered %s %s" % (c["rec"]["symbol"], c["tf"]), a["offered"], b["offered"], bad)
                diff("live ready %s %s" % (c["rec"]["symbol"], c["tf"]), a["ready"], b["ready"], bad)
                diff("live reason %s %s" % (c["rec"]["symbol"], c["tf"]), a["reason"], b["reason"], bad)
                n += 4
                print("  live %-6s %-3s bars python %4d rust %4d" % (c["rec"]["symbol"], c["tf"], len(a["bars"]), len(b["bars"])))
            diff("live refresh_distributions", py["distributions"], rs["distributions"], bad)
            diff("live coinbase_prev_close", py["prev"], rs["prev"], bad)
            # a glance is a live price; what it is made of must agree, not the tick
            shape = lambda q: None if q is None else sorted(q)
            diff("live peek_quote shape", [shape(q) for q in py["peeks"]], [shape(q) for q in rs["peeks"]], bad)
            diff("live archive_intraday", py["archived"], rs["archived"], bad)
            n += 4
            # the stored tables the reads left behind
            import sqlite3
            def table(path, sql):
                con = sqlite3.connect(path)
                rows = con.execute(sql).fetchall()
                con.close()
                return rows
            for sql, label in (("SELECT symbol, ex_date, pay_date, amount, currency FROM distributions WHERE symbol IN ('ZEB','ENB') ORDER BY symbol, ex_date", "distributions"),
                               ("SELECT symbol, tf, start_ts FROM bar_fetches ORDER BY symbol, tf", "bar_fetches"),
                               ("SELECT key, value FROM meta WHERE key LIKE 'coinbase_prev:%' OR key LIKE 'bars_source:%' OR key LIKE 'coinbase_product:%' ORDER BY key", "meta")):
                try:
                    diff("live table %s" % label, table(os.path.join(py_home, "bagholder.db"), sql), table(os.path.join(rs_home, "bagholder.db"), sql), bad)
                except sqlite3.Error as e:
                    bad.append("live table %s: %s" % (label, e))
                n += 1
        finally:
            shutil.rmtree(work, ignore_errors=True)

    print("%d comparisons, %d differing" % (n, len(bad)))
    for b in bad[:25]:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
