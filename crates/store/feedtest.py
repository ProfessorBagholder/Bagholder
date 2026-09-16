"""Exposure records, the watchlist, news, filings, short selling, the gauges
and the notifications: written and read back through both implementations, and
the tables compared.

The cases are the ones the rules exist for: following a listing twice, a news
item with no kind, a second source's filings leaving the first's alone, a
refresh keeping what was read out of a document, a short read that does not
carry a run of reports, and the same notification key raised twice.

    cargo build -p bagholder-store && python3 crates/store/feedtest.py
"""
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)
BIN = os.path.join(ROOT, "target", "debug", "storetool")

NOW = "2026-09-16T12:00:00Z"

EXPOSURES = [
    ["share:AAA:", {"sectors": {"Information Technology": 0.6, "Energy": 0.4},
                    "countries": {"Canada": 1.0}, "coverage": 1.0, "source": "tmx",
                    "asOf": "2026-08-31", "industry": "Software", "error": ""}],
    ["share:BBB::US", {"sectors": {}, "countries": {}, "coverage": 0, "source": "",
                       "asOf": "", "industry": "", "error": "not found"}],
    ["share:AAA:", {"sectors": {"Energy": 1.0}, "countries": {}, "coverage": 0.5,
                    "source": "tmx", "asOf": "2026-09-01"}],       # replaces the first
]

WATCH = [
    {"symbol": "qnc", "exchange": "tsx", "name": "QNC Inc", "currency": "cad", "securityId": "sec-1"},
    {"symbol": "QNC", "exchange": "TSX", "name": "", "currency": "", "securityId": ""},   # keeps its place
    {"symbol": "ZZZ", "exchange": "", "name": "Zed", "currency": "USD", "securityId": ""},
    {"symbol": "  ", "exchange": "TSX", "name": "nothing", "currency": "", "securityId": ""},
]
UNWATCH = [["ZZZ", ""], ["NOPE", "TSX"]]

NEWS = [
    {"symbol": "QNC", "exchange": "TSX", "source": "tmx", "rows": [
        {"id": "n1", "headline": "QNC announces results", "source": "Newswire",
         "url": "https://x/1", "publishedAt": "2026-09-15T12:00:00Z", "kind": "release"},
        {"id": "n2", "headline": "A column about QNC", "source": "The Paper",
         "url": "https://x/2", "publishedAt": "2026-09-14T12:00:00Z"},
        {"id": "", "headline": "no id"},
    ]},
    {"symbol": "ZZZ", "exchange": "", "source": "tmx", "rows": [
        {"id": "n3", "headline": "Zed news", "source": "Wire", "publishedAt": "2026-09-13T12:00:00Z"},
    ]},
]

FILINGS = [
    {"symbol": "QNC", "source": "sedar", "items": [
        {"id": "f1", "category": "Financial", "profileNo": "123", "issuer": "QNC Inc",
         "type": "AIF", "title": "Annual information form", "date": "2026-06-30",
         "dateText": "Jun 30, 2026", "size": "1.2 MB", "url": "https://x/f1"},
        {"id": "f2", "type": "MD&A", "title": "Q2", "date": "2026-08-14"},
        {"id": "", "title": "no id"},
    ]},
    {"symbol": "QNC", "source": "edgar", "items": [
        {"id": "e1", "type": "6-K", "title": "Report", "date": "2026-07-01"},
    ]},
]
ENRICH = [
    {"symbol": "QNC", "id": "f1", "subject": "The year", "summary": "A summary.", "version": 3},
    {"symbol": "QNC", "id": "f2", "summary": "Only a summary."},
]
# the same source again: f1 keeps its reading, f2 is gone, f3 is new
FILINGS_AGAIN = [
    {"symbol": "QNC", "source": "sedar", "items": [
        {"id": "f1", "type": "AIF", "title": "Annual information form", "date": "2026-06-30"},
        {"id": "f3", "type": "News release", "title": "Results", "date": "2026-09-15"},
    ]},
]

SHORTS = [
    {"symbol": "qnc", "exchange": "tsx", "version": 2, "rec": {
        "market": "TSX", "asOf": "2026-09-15", "shares": 1234567, "previous": 1200000,
        "previousOf": "2026-08-31", "change": 34567, "float": 50000000, "ofFloat": 2.47,
        "averageVolume": 250000, "daysToCover": 4.9, "volumeOf": "2026-09-15",
        "volumeSpan": "day", "shortVolume": 10000, "totalVolume": 40000, "volumePct": 25.0,
        "name": "QNC Inc", "series": [{"date": "2026-08-31", "shares": 1200000}]}},
    # a later read with no run of reports: the stored one stays
    {"symbol": "QNC", "exchange": "TSX", "version": 3, "rec": {
        "market": "TSX", "asOf": "2026-09-16", "shares": 1300000, "name": "QNC Inc"}},
]

GAUGES = [
    {"name": "Fear & Greed", "version": 1, "rec": {
        "source": "cnn", "score": 42.5, "rating": "Fear", "asOf": "2026-09-16",
        "indicators": [{"name": "Momentum", "score": 30}], "year": [1, 2, 3]}},
    {"name": "fear & greed", "version": 2, "rec": {"source": "cnn", "score": 55, "rating": "Neutral"}},
]

NOTIFICATIONS = [
    {"kind": "connection", "key": "session:1", "title": "Sign in needed", "body": "Connect again.",
     "extra": {"why": "expired"}, "seen": False},
    {"kind": "connection", "key": "session:1", "title": "Again", "body": "Ignored."},
    {"kind": "order", "key": "order:o1", "title": "Filled", "body": "100 AAA", "seen": True},
]


def tables(path):
    c = sqlite3.connect(path)
    out = {}
    for t in ("exposures", "watchlist", "news", "filings", "shorts", "gauges", "notifications", "universes", "meta"):
        cols = [r[1] for r in c.execute(f"PRAGMA table_info({t})")]
        order = ", ".join(cols)
        out[t] = list(c.execute(f"SELECT {order} FROM {t} ORDER BY {order}"))
    c.close()
    return out


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


FILING_STAMPS = [["QNC", "00012345"], ["ZZZ", ""]]
READ_IDS = [1, 99]

UNIVERSES = [
    {"key": "gainers", "rows": [
        {"symbol": "AAA", "name": "Triple A", "value": 12.5, "percentChange": 3.2,
         "sector": "Energy", "country": "Canada"},
        {"symbol": "BBB", "value": None, "percentChange": None},
        {"symbol": "", "name": "dropped"},
    ]},
]


def python_side(home):
    import store
    store._now_iso = lambda: NOW
    store.set_home(home)
    store.ensure()
    for key, rec in EXPOSURES:
        store.replace_exposure(key, rec)
    added = [store.add_watch(w["symbol"], w["exchange"], w["name"], w["currency"], w["securityId"], NOW)
             for w in WATCH]
    removed = [store.remove_watch(a, b) for a, b in UNWATCH]
    for n in NEWS:
        store.replace_news(n["symbol"], n["exchange"], n["source"], n["rows"], NOW)
    for f in FILINGS:
        store.replace_filings(f["symbol"], f["source"], f["items"], NOW)
    for e in ENRICH:
        store.set_filing_enrichment(e["symbol"], e["id"], e.get("subject"), e.get("summary"), e.get("version"))
    for f in FILINGS_AGAIN:
        store.replace_filings(f["symbol"], f["source"], f["items"], NOW)
    for s in SHORTS:
        store.save_shorts(s["symbol"], s["exchange"], s["rec"], NOW, s["version"])
    for g in GAUGES:
        store.save_gauge(g["name"], g["rec"], NOW, g["version"])
    notes = [store.add_notification(n["kind"], n["key"], n["title"], n["body"],
                                    n.get("extra"), n.get("seen", False)) for n in NOTIFICATIONS]
    marked = store.mark_notifications_seen([1, 2, 99])
    for u in UNIVERSES:
        store.replace_universe(u["key"], u["rows"], NOW)
    for sym, prof in FILING_STAMPS:
        store.mark_filings_fetched(sym, prof, NOW)
    read_marked = store.mark_notifications_read(READ_IDS)
    out = {
        "dividendSymbols": store.dividend_symbols(),
        "allShorts": store.all_shorts(),
        "filingsFetched": store.filings_fetched_at(),
        "filingsFetchedFor": store.filings_fetched_at("QNC"),
        "sedarProfile": store.sedar_profile("QNC"),
        "soldSince": store.sold_since("acct-1", "sec-1", "2026-01-01T00:00:00Z", "AAA"),
        "positionQuantity": store.position_quantity("acct-1", "sec-1"),
        "balancesCount": store.balances_count(),
        "latestNotificationId": store.latest_notification_id(),
        "unreadNotifications": store.unread_notifications(),
        "readMarked": read_marked,
        "added": [a for a in added],
        "removed": removed,
        "watchlist": store.list_watchlist(),
        "newsIds": sorted(store.news_ids("QNC", "TSX")),
        "hasRelease": store.has_wire_release("QNC"),
        "newsFetched": store.news_fetched_at(),
        "filings": store.filings("QNC"),
        "filingsAll": store.filings(),
        "shorts": store.shorts_for("QNC", "TSX"),
        "gauge": store.gauge("Fear & Greed"),
        "notifications": notes,
        "list": store.list_notifications(0, "", False, 50, False),
        "unseen": store.list_notifications(0, "", True, 50, False),
        "marked": marked,
    }
    store.close_all()
    return out


def main():
    work = tempfile.mkdtemp(prefix="feedtest-")
    pyhome = os.path.join(work, "py")
    rshome = os.path.join(work, "rs")
    os.makedirs(pyhome)
    os.makedirs(rshome)

    want = python_side(pyhome)

    import store
    store.set_home(rshome)
    store.ensure()
    store.close_all()

    payload = {"now": NOW, "exposures": EXPOSURES, "watch": WATCH, "unwatch": UNWATCH,
               "news": NEWS, "filings": FILINGS + FILINGS_AGAIN, "enrich": ENRICH,
               "shorts": SHORTS, "gauges": GAUGES, "notifications": NOTIFICATIONS,
               "seen": [1, 2, 99], "universes": UNIVERSES,
               "newsSymbol": "QNC", "newsExchange": "TSX", "filingSymbol": "QNC",
               "shortSymbol": "QNC", "shortExchange": "TSX", "gaugeName": "Fear & Greed",
               "filingStamps": FILING_STAMPS, "read": READ_IDS,
               "soldAccount": "acct-1", "soldSecurity": "sec-1", "soldSince": "2026-01-01T00:00:00Z",
               "soldSymbol": "AAA"}
    # the filings are applied in one pass on the Rust side, so the enrichment has
    # to run between them the same way; the tool takes them in the order given
    payload["filings"] = FILINGS
    r = subprocess.run([BIN, "feeds", os.path.join(rshome, "bagholder.db")],
                       input=json.dumps(payload), capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stderr[-2000:])
        return 1
    # the second filing pass, after the enrichment, as Python did it
    payload2 = dict(payload, filings=FILINGS_AGAIN, enrich=[], watch=[], unwatch=[], news=[],
                    shorts=[], gauges=[], notifications=[], seen=[], universes=[], exposures=[],
                    filingStamps=[], read=[])
    r2 = subprocess.run([BIN, "feeds", os.path.join(rshome, "bagholder.db")],
                        input=json.dumps(payload2), capture_output=True, text=True)
    if r2.returncode != 0:
        print(r2.stderr[-2000:])
        return 1
    got = json.loads(r2.stdout)
    first = json.loads(r.stdout)
    # what only the first pass produced
    for k in ("added", "removed", "notifications", "marked", "readMarked"):
        got[k] = first[k]

    bad = []
    got["newsIds"] = sorted(got["newsIds"])
    for key in want:
        if norm(want[key]) != norm(got.get(key)):
            bad.append(f"{key}:\n    py={json.dumps(norm(want[key]))[:300]}\n    rs={json.dumps(norm(got.get(key)))[:300]}")

    a, b = tables(os.path.join(pyhome, "bagholder.db")), tables(os.path.join(rshome, "bagholder.db"))
    for t in a:
        if a[t] != b[t]:
            shown = False
            for x, y in zip(a[t], b[t]):
                if x != y:
                    bad.append(f"table {t}:\n    py={x}\n    rs={y}")
                    shown = True
                    break
            if not shown:
                bad.append(f"table {t}: {len(a[t])} rows py, {len(b[t])} rs")

    for line in bad[:20]:
        print("  " + line)
    print(f"{len(a)} feed tables, {len(bad)} differences")
    shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
