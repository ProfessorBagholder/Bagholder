"""Differential test for exposure.py and the symbol search against their Rust
ports: the HTML table reader and every issuer's page parser on real issuer
pages and hand-made markup, iShares' holdings CSVs, Yahoo's summary, the name
tables, the search parsers and ranking -- and, live, both classifying the
book's own securities into their own copy of the database, look-through and
all, and running the same searches.
"""
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")))
import bagholder  # noqa: E402
import exposure  # noqa: E402
import store  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "..", "target", "release", "exposuretool")
FIX = os.environ.get("EXPOSURE_DIR") or "/tmp/exposure"


def run_rust(payload, mode=""):
    p = subprocess.run([TOOL] + ([mode] if mode else []), input=json.dumps(payload), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("exposuretool failed: %s" % p.stderr[-3000:])
    return json.loads(p.stdout)


def close(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(a - b) <= 1e-9 * max(1.0, abs(a), abs(b))
    if isinstance(a, dict) and isinstance(b, dict):
        return list(a.keys()) == list(b.keys()) and all(close(a[k], b[k]) for k in a)
    if isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)):
        return len(a) == len(b) and all(close(x, y) for x, y in zip(a, b))
    return a == b


def diff(label, want, got, bad):
    if not close(want, got):
        bad.append("%s\n  python: %s\n  rust:   %s" % (label, json.dumps(want, default=str)[:700], json.dumps(got, default=str)[:700]))


def main():
    rnd = random.Random(20260916)
    fx = json.load(open(os.path.join(FIX, "fixtures.json")))
    bad = []
    html = list(fx["html"]) + [
        "<TABLE><tr><TH>Name</th><th>Ticker</th><th>Weight</th><th>Sector</th><th>Country</th></tr>"
        "<tr><td>Apple &amp; Co<br/>Inc</td><td>AAPL US</td><td>12.5%</td><td>Technology</td><td></td></tr>"
        "<tr><td>Cash and other</td><td>CASH</td><td>1</td></tr><tr></tr>"
        "<tr><td>Bank</td><td>RY CN</td><td>(3.0)</td></tr><tr><td>Shopify</td><td>SHOP CT</td><td>7</td><td>Tech</td><td>Canada</td></tr></table>",
        "<table><tr><td>Reference Asset</td><td>NVDA</td></tr></table><table><tr><th>Holdings</th><th>%</th></tr><tr><td>Vanguard ETF</td><td>50</td></tr></table>",
        "<table><tr><td a='>'>x<td>y</tr><!-- <table> --><script>var t='<table>';</script></table><p>tail",
        "<table><tr><td>unclosed", "", "no tables", "<table><tr><td>&notanentity; &#x2019; </td></tr></table>",
        "<div>Ticker ** ABCD:TSX</div> <b>Underlying Stock**</b> Cameco Corp. (CCO:TSX) and more",
        "var portfolioBreakdownData = {\"data\": {\"sector\": [{\"name\": \"Technology\", \"weight\": \"45.5\"}, {\"name\": \"cash\", \"weight\": 5}]}};\n"
        "var holdingsData = {\"data\": [{\"ticker\": \"MSFT US Equity\", \"weight_percent\": \"10\", \"security_name\": \"Microsoft\", \"gics_sector\": \"Information Technology\", \"country\": \"\"},"
        " {\"ticker\": \"XIU\", \"weight_percent\": 5, \"security_name\": \"iShares S&P/TSX 60 Index ETF\", \"country\": \"XIU\"}, {\"ticker\": \"\", \"weight_percent\": 5}]};\n",
    ]
    csvs = list(fx["csv"]) + ["", "Fund Holdings as of,\"Sep 15, 2026\"\nTicker,Name,Sector,Asset Class,Market Value,Weight (%),Location,Exchange,Currency\n"
                              "RY,ROYAL BANK,Financials,Equity,1,\"6.5\",Canada,Toronto Stock Exchange,CAD\nCASH,CAD CASH,Cash,Cash,1,0.1,Canada,-,CAD\n"
                              ",BLANK,x,Equity,1,1,Canada,x,CAD\nXEF,ISHARES CORE MSCI EAFE,Other,Equity,1,20,Canada,TSX,CAD\nSHORT,ROW"]
    yahoo = [{"quoteSummary": {"result": [{"topHoldings": {"sectorWeightings": [{"technology": {"raw": 0.3}}, {"realestate": {"raw": 0.1}}, {"cash": 0.05}],
                                                           "holdings": [{"symbol": "RY.TO", "holdingName": "Royal Bank", "holdingPercent": {"raw": 0.07}},
                                                                        {"symbol": "XEF.TO", "holdingName": "iShares Core MSCI EAFE ETF", "holdingPercent": 0.2},
                                                                        {"symbol": "", "holdingPercent": 0.1}]}}]}}, {}, {"quoteSummary": {"result": []}}]
    countries = ["USA", "u.s.", "Korea, Republic of", "broad", "", "Germany", "TSX", "nasdaq global select", " Canada ", "CBOE CANADA"]
    nasdaq = [{"data": [{"symbol": "aapl", "name": "Apple Inc. Common Stock", "exchange": "NASDAQ-GS", "asset": "STOCKS"},
                        {"symbol": "BRK.B", "name": "Berkshire", "exchange": "NYSE", "asset": "STOCKS"}, {"symbol": "XYZ.WS", "exchange": "NYSE", "asset": "STOCKS"},
                        {"symbol": "SPY", "name": "SPDR, Common Shares", "exchange": "AMEX", "asset": "ETF"}, {"symbol": "X", "exchange": "LSE", "asset": "STOCKS"},
                        {"symbol": "F", "exchange": "NYSE", "asset": "INDEX"}, "junk"]}, [], {"data": None}]
    tsx = [{"results": [{"symbol": "ry", "name": "Royal Bank"}, {"symbol": ""}, "x"]}, {"results": None}, []]
    rows = [{"symbol": s, "exchange": e, "name": s} for s, e in [("RY", "TSX"), ("RYA", "TSX"), ("ARY", "NYSE"), ("RY", "TSX"), ("RY", "NYSE")]] + [{"symbol": "CL", "exchange": "NYMEX", "rank": 0}, {"symbol": "RYX", "exchange": "TSX", "rank": None}]
    ranks = [["ry", rows], ["", rows], ["WTI", rows]] + [["R", [{"symbol": "R%d" % i, "exchange": "TSX"} for i in range(20)]]]

    got = run_rust({"html": html, "csv": csvs, "yahoo": yahoo, "countries": countries, "nasdaq": nasdaq, "tsx": tsx, "ranks": ranks})
    for i, h in enumerate(html):
        t = exposure.html_tables(h)
        diff("html_tables[%d]" % i, t, got["tables"][i], bad)
        diff("parse_harvest_tables[%d]" % i, list(exposure.parse_harvest_tables(t)), got["harvest"][i], bad)
        diff("parse_ninepoint_page[%d]" % i, list(exposure.parse_ninepoint_page(h)), got["ninepoint"][i], bad)
        diff("parse_evolve_page[%d]" % i, list(exposure.parse_evolve_page(h)), got["evolve"][i], bad)
    for i, c in enumerate(csvs):
        diff("parse_ishares_csv[%d]" % i, list(exposure.parse_ishares_csv(c)), got["ishares"][i], bad)
    for i, d in enumerate(yahoo):
        diff("parse_yahoo_summary[%d]" % i, list(exposure.parse_yahoo_summary(d)), got["yahoo"][i], bad)
    for i, c in enumerate(countries):
        diff("norm/venue_country(%r)" % c, [exposure.norm_country(c), exposure.venue_country(c)], got["countries"][i], bad)
    for i, d in enumerate(nasdaq):
        diff("parse_nasdaq_search[%d]" % i, bagholder.parse_nasdaq_search(d), got["nasdaq"][i], bad)
    for i, d in enumerate(tsx):
        diff("parse_tsx_search[%d]" % i, bagholder.parse_tsx_search(d, "TSX"), got["tsx"][i], bad)
    for i, (text, rs) in enumerate(ranks):
        diff("rank_search[%d]" % i, bagholder.rank_search(text, [dict(r) for r in rs]), got["ranks"][i], bad)
    n = len(html) * 4 + len(csvs) + len(yahoo) + len(countries) + len(nasdaq) + len(tsx) + len(ranks)
    tables = sum(len(x) for x in got["tables"])
    print("  %d pages (%d tables), %d CSVs" % (len(html), tables, len(csvs)))

    if "--live" in sys.argv:
        live = os.environ.get("BAGHOLDER_DB") or os.path.expanduser("~/.bagholder/bagholder.db")
        work = tempfile.mkdtemp(prefix="exposuretest-")
        try:
            py_home, rs_home = os.path.join(work, "py"), os.path.join(work, "rs")
            os.makedirs(py_home)
            os.makedirs(rs_home)
            shutil.copy(live, os.path.join(py_home, "bagholder.db"))
            import sqlite3
            con = sqlite3.connect(os.path.join(py_home, "bagholder.db"))
            con.execute("DELETE FROM exposures")
            con.commit()
            con.close()
            shutil.copy(os.path.join(py_home, "bagholder.db"), os.path.join(rs_home, "bagholder.db"))
            store.set_home(py_home)
            secs = [s for s in store.list_securities() if s.get("symbol") and not s["id"].startswith("sec-c-") and not s["id"].startswith("sec-o-")]
            rnd.shuffle(secs)
            secs = secs[:14]
            shares = [["SHOP", "TSX", "CAD"], ["AAPL", "NASDAQ", "USD"], ["F", "", ""], ["XYZQ", "CSE", "CAD"]]
            searches = ["shopify", "ry", "BTQ", "royal bank of canada", "zzzzqqq", "YES.V"]
            py = {"securities": [exposure.refresh_security(s) for s in secs],
                  "shares": [exposure.share_exposure(*r) for r in shares],
                  "searches": [bagholder.symbol_search(t) for t in searches]}
            rs = run_rust({"db": os.path.join(rs_home, "bagholder.db"), "securities": secs, "shares": shares, "searches": searches}, "live")
            for i, s in enumerate(secs):
                diff("live refresh_security %s (%s)" % (s["symbol"], s.get("name")), py["securities"][i], rs["securities"][i], bad)
                print("  live %-8s %-40s %s" % (s["symbol"], (s.get("name") or "")[:40], py["securities"][i].get("source")))
            for i, r in enumerate(shares):
                diff("live share_exposure %s" % r, py["shares"][i], rs["shares"][i], bad)
            for i, t in enumerate(searches):
                diff("live symbol_search %r" % t, py["searches"][i], rs["searches"][i], bad)
            n += len(secs) + len(shares) + len(searches)

            def rows(path):
                con = sqlite3.connect(path)
                out = con.execute("SELECT key, sectors, countries, coverage, source, as_of, industry, error FROM exposures ORDER BY key").fetchall()
                con.close()
                return [list(r[:1]) + [json.loads(r[1] or "{}"), json.loads(r[2] or "{}")] + list(r[3:]) for r in out]
            a, b = rows(os.path.join(py_home, "bagholder.db")), rows(os.path.join(rs_home, "bagholder.db"))
            diff("live exposures table (%d rows)" % len(a), a, b, bad)
            n += 1
        finally:
            shutil.rmtree(work, ignore_errors=True)

    print("%d comparisons, %d differing" % (n, len(bad)))
    for b_ in bad[:25]:
        print(b_)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
