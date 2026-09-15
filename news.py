"""News for the symbols the book holds and watches, from two public per-symbol sources:
TMX Money's news for Canadian listings and Nasdaq's for US ones. Each item is tagged with
the symbol it was read for; nothing is guessed from headlines."""
from __future__ import annotations

import html
import json
import re
import sys
import time
from datetime import datetime, timedelta, timezone

import market
import store

TMX_NEWS_QUERY = ("query getNewsForSymbol($symbol: String!, $page: Int!, $limit: Int!, $locale: String!) "
                  "{ news: getNewsForSymbol(symbol: $symbol, page: $page, limit: $limit, locale: $locale) { headline datetime source newsid summary } }")
TMX_NEWS_URL = "https://money.tmx.com/en/quote/%s/news/%s"
NASDAQ_NEWS_URL = "https://api.nasdaq.com/api/news/topic/articlebysymbol?q=%s|STOCKS&offset=0&limit=%d"
NASDAQ_LATEST_URL = "https://api.nasdaq.com/api/news/topic/latestnews?offset=0&limit=%d"
NASDAQ_PRESS_URL = "https://api.nasdaq.com/api/news/topic/press_release?q=symbol:%s|assetclass:stocks&offset=0&limit=%d"   # a US listing's own releases, beside its news
WIRE_MARKS = ("wire", "newsfile", "cision", "cnw")   # in a source's name: GlobeNewswire, Business Wire, PR Newswire, ACCESS Newswire, TheNewsWire, Canada Newswire, TMX Newsfile, Marketwired
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36"
NASDAQ_HEADERS = {"User-Agent": UA, "Accept": "application/json, text/plain, */*", "Origin": "https://www.nasdaq.com", "Referer": "https://www.nasdaq.com/"}
TMX_HEADERS = {"User-Agent": UA, "locale": "en", "Origin": "https://money.tmx.com", "Referer": "https://money.tmx.com/"}
PER_SYMBOL = 12
# the market-wide feed is a listing of its own: Nasdaq's latest news, whatever it names
MARKET = ("*", "MARKET", "")
PER_MARKET = 50
FRESH_MINUTES = 15
KEEP = 400            # items kept in the database, newest first

_last_call = {}


def _pace(host, seconds=0.6):
    wait = _last_call.get(host, 0) + seconds - time.time()
    if wait > 0:
        time.sleep(wait)
    _last_call[host] = time.time()


def _s(v):
    return "" if v is None else str(v)


def kind_of(source):
    """What an item is, told by where it came from: a wire carries the company's own
    release, a publisher writes a story about it."""
    s = _s(source).lower()
    return "release" if any(m in s for m in WIRE_MARKS) else "story"


def clean_text(t):
    return re.sub(r"\s+", " ", html.unescape(_s(t))).strip()


def parse_tmx_news(data, symbol):
    """TMX's items for a symbol (in the venue's form, as `tmx_quote_symbol` gives it) into news
    rows: the headline, its exact time, the wire it came on, and TMX's page for it."""
    rows = []
    for it in ((data or {}).get("data") or {}).get("news") or []:
        if not isinstance(it, dict) or not it.get("newsid"):
            continue
        when = _s(it.get("datetime"))
        try:
            ts = datetime.fromisoformat(when).astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        except ValueError:
            continue
        source = clean_text(it.get("source")).replace(" via QuoteMedia", "")
        rows.append({"id": "tmx:%s" % it["newsid"], "headline": clean_text(it.get("headline")), "source": source,
                     "url": TMX_NEWS_URL % (symbol, it["newsid"]), "publishedAt": ts, "kind": kind_of(source)})
    return rows


_AGO = re.compile(r"(\d+)\s+(minute|hour|day)s?\s+ago", re.I)


def nasdaq_when(row, now):
    """Nasdaq gives a day and an age ('17 minutes ago'): the time is the age taken off now,
    to the minute; older items keep the day alone at midnight UTC."""
    m = _AGO.search(_s(row.get("ago")))
    if m:
        n, unit = int(m.group(1)), m.group(2).lower()
        delta = timedelta(minutes=n) if unit == "minute" else timedelta(hours=n) if unit == "hour" else timedelta(days=n)
        return (now - delta).strftime("%Y-%m-%dT%H:%M:00Z")
    try:
        return datetime.strptime(_s(row.get("created")), "%b %d, %Y").strftime("%Y-%m-%dT00:00:00Z")
    except ValueError:
        return ""


def parse_nasdaq_news(data, now=None, symbol="", kind=None):
    """Nasdaq pads a symbol's feed with market-wide pieces; an item is kept only when the
    symbol is among the ones Nasdaq itself lists for it (a feed asked without a symbol keeps
    all). An item's kind is the feed's when it has one (the press-release feed), else told
    by its publisher; a release Nasdaq names no wire for reads as Nasdaq's."""
    now = now or datetime.now(timezone.utc)
    want = _s(symbol).strip().lower()
    rows = []
    for it in ((data or {}).get("data") or {}).get("rows") or []:
        if not isinstance(it, dict) or not it.get("id") or not it.get("title"):
            continue
        if want:
            named = {_s(x).split("|")[0].strip().lower() for x in (it.get("related_symbols") or [])} | {_s(it.get("primarysymbol")).strip().lower()}
            if want not in named:
                continue
        when = nasdaq_when(it, now)
        if not when:
            continue
        url = _s(it.get("url"))
        source = clean_text(it.get("publisher")) or ("Nasdaq" if kind == "release" else "")
        rows.append({"id": "nasdaq:%s" % it["id"], "headline": clean_text(it.get("title")), "source": source,
                     "url": url if url.startswith("http") else "https://www.nasdaq.com" + url, "publishedAt": when, "kind": kind or kind_of(source)})
    return rows


def source_for(symbol, exchange, currency):
    """Which wire answers for a listing: TMX for the Canadian venues it carries, Nasdaq for US ones and for the market feed."""
    if (symbol, _s(exchange).upper()) == (MARKET[0], MARKET[1]):
        return "nasdaq"
    form = market.tmx_form(exchange, currency)
    if form == ":US":
        return "nasdaq"
    if form is None:
        return ""
    return "tmx"


def fetch_symbol(symbol, exchange, currency, ssl_context=None, now=None):
    """The latest items for one listing from its wire, as rows; [] when the wire has none or fails."""
    src = source_for(symbol, exchange, currency)
    sym = market.tmx_symbol(symbol)
    if not src or not sym:
        return src, []
    try:
        if symbol == MARKET[0]:
            _pace("api.nasdaq.com")
            text = market._get_text(NASDAQ_LATEST_URL % PER_MARKET, ssl_context, headers=NASDAQ_HEADERS)
            return src, parse_nasdaq_news(json.loads(text), now, "")
        if src == "tmx":
            # TMX names a listing by its venue, and the news query answers nothing under the wrong
            # name: the same code the quote asks under, through the same lookup, so a record with a
            # wrong or missing venue resolves here as it does everywhere else and is remembered once
            code = market.tmx_quote_symbol(symbol, exchange, currency)
            if not code:
                return src, []
            def ask(form):
                _pace("app-money.tmx.com")
                data = market._post_json("https://app-money.tmx.com/graphql",
                                         {"operationName": "getNewsForSymbol", "variables": {"symbol": form, "page": 1, "limit": PER_SYMBOL, "locale": "en"}, "query": TMX_NEWS_QUERY},
                                         ssl_context, TMX_HEADERS)
                return parse_tmx_news(data, form)
            return src, market.tmx_lookup(code, ask, ssl_context)[0]
        _pace("api.nasdaq.com")
        text = market._get_text(NASDAQ_NEWS_URL % (sym, PER_SYMBOL), ssl_context, headers=NASDAQ_HEADERS)
        rows = parse_nasdaq_news(json.loads(text), now, sym)
        # the listing's own releases come on a feed of their own; each once, beside the stories
        try:
            _pace("api.nasdaq.com")
            text = market._get_text(NASDAQ_PRESS_URL % (sym, PER_SYMBOL), ssl_context, headers=NASDAQ_HEADERS)
            seen = {r["id"] for r in rows}
            rows.extend(r for r in parse_nasdaq_news(json.loads(text), now, sym, kind="release") if r["id"] not in seen)
        except Exception as e:
            sys.stderr.write("bagholder news: %s releases from nasdaq failed: %s\n" % (sym, e))
        return src, rows
    except Exception as e:
        sys.stderr.write("bagholder news: %s from %s failed: %s\n" % (sym, src, e))
        return src, None


def stale(listings, now=None, minutes=FRESH_MINUTES):
    """The listings whose news is older than `minutes`, as (symbol, exchange, currency)."""
    now = now or datetime.now(timezone.utc)
    fetched = store.news_fetched_at()
    out = []
    for symbol, exchange, currency in listings:
        key = store.news_key(symbol, exchange)
        last = fetched.get(key) or ""
        try:
            age = now - datetime.fromisoformat(last.replace("Z", "+00:00")) if last else None
        except ValueError:
            age = None
        if age is None or age > timedelta(minutes=minutes):
            out.append((symbol, exchange, currency))
    return out


def refresh(listings, ssl_context=None, now=None):
    """Read the wire for every stale listing; each answer replaces that listing's rows. Returns how many answered."""
    done = 0
    for symbol, exchange, currency in stale(listings, now=now):
        src, rows = fetch_symbol(symbol, exchange, currency, ssl_context, now)
        if rows is None:
            continue
        store.replace_news(symbol, exchange, src, rows, now=now)
        done += 1
    if done:
        store.trim_news(KEEP)
    return done
