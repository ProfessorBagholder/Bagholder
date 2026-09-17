"""Differential test for news.py against crates/market/src/news.rs.

Both implementations are handed the same wire answers -- the shapes TMX and
Nasdaq actually send, plus the edges each parser has to refuse -- and their
rows are compared field by field. With `--live` the two also read the real
wires for a handful of listings and the answers are compared by id.
"""
import json
import os
import subprocess
import sys
from datetime import datetime, timezone

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")))
import news  # noqa: E402

TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "target", "release", "newstool")

NOW = datetime(2026, 9, 16, 14, 30, 0, tzinfo=timezone.utc)
NOW_UNIX = int(NOW.timestamp())


def tmx_case(symbol, items):
    return {"symbol": symbol, "data": {"data": {"news": items}}}


def nasdaq_case(symbol, rows, kind=None):
    c = {"symbol": symbol, "data": {"data": {"rows": rows}}, "now": NOW_UNIX}
    if kind:
        c["kind"] = kind
    return c


TMX = [
    tmx_case("SHOP", [
        {"newsid": "1001", "headline": "Shopify &amp; partners  announce\nsomething", "source": "GlobeNewswire via QuoteMedia",
         "datetime": "2026-09-16T09:31:00-04:00", "summary": "x"},
        {"newsid": "1002", "headline": "Analysts weigh in", "source": "The Globe and Mail",
         "datetime": "2026-09-15T18:00:00Z"},
    ]),
    # refused: no id, no parsable time, not an object
    tmx_case("BCE", [
        {"headline": "no id", "source": "Newsfile Corp", "datetime": "2026-09-16T09:31:00-04:00"},
        {"newsid": "2001", "headline": "bad time", "source": "Cision", "datetime": "not a time"},
        {"newsid": "2002", "headline": "no time at all", "source": "CNW Group", "datetime": ""},
        "not a dict",
    ]),
    tmx_case("ATD", []),
    {"symbol": "X", "data": {}},
    {"symbol": "X", "data": None},
]

NASDAQ = [
    nasdaq_case("AAPL", [
        {"id": "5001", "title": "Apple ships a thing", "publisher": "Business Wire", "ago": "17 minutes ago",
         "created": "Sep 16, 2026", "url": "/articles/apple", "related_symbols": ["AAPL|STOCKS", "MSFT"]},
        {"id": "5002", "title": "Market wrap", "publisher": "Zacks", "ago": "", "created": "Sep 15, 2026",
         "url": "https://www.zacks.com/x", "related_symbols": [], "primarysymbol": "AAPL"},
        # padded in, belongs to nobody this feed was asked for
        {"id": "5003", "title": "Unrelated", "publisher": "Zacks", "ago": "2 hours ago", "created": "Sep 16, 2026",
         "url": "/x", "related_symbols": ["TSLA"], "primarysymbol": "TSLA"},
        # refused
        {"title": "no id", "ago": "1 day ago", "created": "Sep 16, 2026", "primarysymbol": "AAPL"},
        {"id": "5005", "ago": "1 day ago", "created": "Sep 16, 2026", "primarysymbol": "AAPL"},
        {"id": "5006", "title": "no time", "ago": "", "created": "", "primarysymbol": "AAPL"},
        {"id": "5007", "title": "odd age", "ago": "about a minute ago", "created": "Sep 14, 2026",
         "primarysymbol": "AAPL", "url": "/y"},
        {"id": "5008", "title": "weeks", "ago": "3 weeks ago", "created": "Aug 26, 2026",
         "primarysymbol": "AAPL", "url": "/z"},
        {"id": "5009", "title": "no publisher release", "ago": "5 hours ago", "created": "Sep 16, 2026",
         "primarysymbol": "AAPL", "url": "/w", "publisher": ""},
    ]),
    nasdaq_case("AAPL", [
        {"id": "5009", "title": "no publisher release", "ago": "5 hours ago", "created": "Sep 16, 2026",
         "primarysymbol": "AAPL", "url": "/w", "publisher": ""},
        {"id": "5010", "title": "Apple Reports Results", "ago": "1 day ago", "created": "Sep 15, 2026",
         "primarysymbol": "AAPL", "url": "/r", "publisher": "PR Newswire"},
    ], kind="release"),
    # the market feed: no symbol, everything kept
    nasdaq_case("", [
        {"id": "6001", "title": "Latest one", "publisher": "Reuters", "ago": "3 minutes ago", "created": "Sep 16, 2026", "url": "/a"},
        {"id": "6002", "title": "Latest two", "publisher": "", "ago": "26 hours ago", "created": "Sep 15, 2026", "url": "/b"},
    ]),
    nasdaq_case("AAPL", []),
    {"symbol": "AAPL", "data": {}, "now": NOW_UNIX},
]

WHEN = [
    {"row": r, "now": NOW_UNIX} for r in [
        {"ago": "1 minute ago", "created": "Sep 16, 2026"},
        {"ago": "59 minutes ago", "created": "Sep 16, 2026"},
        {"ago": "1 hour ago", "created": "Sep 16, 2026"},
        {"ago": "36 hours ago", "created": "Sep 15, 2026"},
        {"ago": "2 days ago", "created": "Sep 14, 2026"},
        {"ago": "900 days ago", "created": "Jan 01, 2024"},
        {"ago": "17 Minutes Ago", "created": "Sep 16, 2026"},
        {"ago": "posted 4 hours ago by staff", "created": "Sep 16, 2026"},
        {"ago": "", "created": "Jan 01, 2026"},
        {"ago": "", "created": "Dec 31, 2025"},
        {"ago": "", "created": "Feb 29, 2024"},
        {"ago": "", "created": "nonsense"},
        {"ago": "", "created": ""},
        {"ago": "3 weeks ago", "created": "Aug 26, 2026"},
        {"ago": "minutes ago", "created": "Sep 16, 2026"},
        {"ago": "12minutes ago", "created": "Sep 16, 2026"},
        {"ago": "12 minutesago", "created": "Sep 16, 2026"},
        {"ago": "2 hours ago and 5 minutes ago", "created": "Sep 16, 2026"},
        {"ago": "5 minutes ago and 2 hours ago", "created": "Sep 16, 2026"},
        {"ago": None, "created": None},
    ]
]

KINDS = ["GlobeNewswire", "Business Wire", "PR Newswire", "ACCESS Newswire", "TheNewsWire",
         "Canada Newswire", "TMX Newsfile", "Marketwired", "Cision", "CNW Group", "cnwgroup",
         "Reuters", "The Globe and Mail", "Zacks", "", "Nasdaq", "WIRE", "newsfileCORP"]

CLEAN = ["  a  b\tc\nd ", "AT&amp;T &lt;tag&gt; &quot;q&quot; &#39;a&#39;", "café &nbsp; bar",
         "&#8217;curly&#x2019;", "&notanentity; x", "", "   ", "&amp;amp;"]

SOURCES = [("*", "MARKET", ""), ("*", "market", ""), ("AAPL", "NASDAQ", "USD"), ("AAPL", "", "USD"),
           ("SHOP", "TSX", "CAD"), ("XYZ", "CSE", "CAD"), ("ABC", "Cboe Canada", "CAD"),
           ("ZZZ", "TSX-V", "CAD"), ("F", "", ""), ("BTC", "CRYPTO", "CAD"), ("X", "LSE", "GBP")]


def run_rust(payload, mode=""):
    p = subprocess.run([TOOL] + ([mode] if mode else []), input=json.dumps(payload), capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit("newstool failed: %s" % p.stderr)
    return json.loads(p.stdout)


def diff(label, want, got, bad):
    if want != got:
        bad.append("%s\n  python: %s\n  rust:   %s" % (label, json.dumps(want), json.dumps(got)))


def main():
    payload = {"now": NOW_UNIX, "tmx": TMX, "nasdaq": NASDAQ, "when": WHEN,
               "kinds": KINDS, "clean": CLEAN, "sources": [list(s) for s in SOURCES]}
    got = run_rust(payload)
    bad = []
    n = 0

    for i, c in enumerate(TMX):
        diff("parse_tmx_news[%d]" % i, news.parse_tmx_news(c["data"], c["symbol"]), got["tmx"][i], bad)
        n += 1
    for i, c in enumerate(NASDAQ):
        want = news.parse_nasdaq_news(c["data"], NOW, c["symbol"], c.get("kind"))
        diff("parse_nasdaq_news[%d]" % i, want, got["nasdaq"][i], bad)
        n += 1
    for i, c in enumerate(WHEN):
        diff("nasdaq_when[%r]" % (c["row"].get("ago"),), news.nasdaq_when(c["row"], NOW), got["when"][i], bad)
        n += 1
    for i, s in enumerate(KINDS):
        diff("kind_of[%r]" % s, news.kind_of(s), got["kinds"][i], bad)
        n += 1
    for i, s in enumerate(CLEAN):
        diff("clean_text[%r]" % s, news.clean_text(s), got["clean"][i], bad)
        n += 1
    for i, s in enumerate(SOURCES):
        diff("source_for%r" % (s,), news.source_for(*s), got["sources"][i], bad)
        n += 1

    if "--fuzz" in sys.argv:
        # the entity reader is the one piece with a table behind it; it is
        # given random entity-shaped noise until it agrees on all of it
        import random
        from html.entities import html5
        names = list(html5)
        rnd = random.Random(20260916)
        words = []
        for _ in range(4000):
            parts = []
            for _ in range(rnd.randint(1, 4)):
                pick = rnd.random()
                if pick < 0.45:
                    nm = rnd.choice(names)
                    if rnd.random() < 0.3:
                        nm = nm.rstrip(";")
                    if rnd.random() < 0.25:
                        nm += rnd.choice(["it", "X", "1", ";", "&"])
                    parts.append("&" + nm)
                elif pick < 0.6:
                    parts.append("&#%d%s" % (rnd.randint(0, 0x11000), rnd.choice([";", ""])))
                elif pick < 0.7:
                    parts.append("&#x%x%s" % (rnd.randint(0, 0x11000), rnd.choice([";", ""])))
                elif pick < 0.8:
                    parts.append("&" + "".join(rnd.choice("abcXY#;& ") for _ in range(rnd.randint(0, 40))))
                else:
                    parts.append(rnd.choice(["&", "&&", " ", "caf\u00e9", "a", ";", "&#;", "&#x;"]))
            words.append("".join(parts))
        out = run_rust({"now": NOW_UNIX, "clean": words})["clean"]
        for i, w in enumerate(words):
            diff("clean_text fuzz[%r]" % w, news.clean_text(w), out[i], bad)
        n += len(words)

    if "--live" in sys.argv:
        db = sys.argv[sys.argv.index("--live") + 1]
        os.environ["BAGHOLDER_HOME"] = os.path.dirname(db)
        listings = [("AAPL", "NASDAQ", "USD"), ("SHOP", "TSX", "CAD"), ("*", "MARKET", "")]
        today = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        rust = run_rust({"db": db, "today": today,
                         "now": datetime.now(timezone.utc).timestamp(),
                         "listings": [list(l) for l in listings]}, "live")
        for i, l in enumerate(listings):
            src, rows = news.fetch_symbol(*l)
            rows = rows or []
            diff("live source %s" % (l,), src, rust[i]["source"], bad)
            pi = sorted(r["id"] for r in rows)
            ri = sorted(r["id"] for r in (rust[i]["rows"] or []))
            # the wires move between the two calls; the overlap must agree exactly
            shared = set(pi) & set(ri)
            if not shared and (pi or ri):
                bad.append("live %s: no overlap\n  python: %s\n  rust: %s" % (l, pi[:3], ri[:3]))
            pm = {r["id"]: r for r in rows}
            rm = {r["id"]: r for r in (rust[i]["rows"] or [])}
            for k in sorted(shared):
                diff("live %s %s" % (l, k), pm[k], rm[k], bad)
            n += 1 + len(shared)
            print("  live %-22s python %3d  rust %3d  shared %3d" % (l, len(pi), len(ri), len(shared)))

    print("%d comparisons, %d differing" % (n, len(bad)))
    for b in bad:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
