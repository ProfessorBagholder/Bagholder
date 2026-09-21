"""The market universes the heatmap can show, from two public sources: the S&P/TSX 60 from
TMX Money (its constituents with their index weights, each quoted for the day's change and
its sector), and the US market from Nasdaq's screener (every US listing with its price,
day change, market cap, sector and country, in one answer), from which the hundred largest
US companies and the hundred largest foreign companies listed in the US are taken."""
from __future__ import annotations

import json
import sys
import time

import exposure
import market
import store

CANADA_INDEX = "^TX60"
TOP = 100
TMX_CONSTITUENTS_QUERY = ("query getIndexConstituents($symbol: String!) { constituents: getIndexConstituents(symbol: $symbol) "
                          "{ symbol quotedMarketValue longName shortName weight exShortName exchange exLongName } }")
TMX_TILE_QUERY = ("query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) "
                  "{ symbol name price percentChange sector } }")
SCREENER_URL = "https://api.nasdaq.com/api/screener/stocks?tableonly=true&limit=25&offset=0&download=true"
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36"
NASDAQ_HEADERS = {"User-Agent": UA, "Accept": "application/json, text/plain, */*", "Origin": "https://www.nasdaq.com", "Referer": "https://www.nasdaq.com/"}
TMX_HEADERS = {"User-Agent": UA, "locale": "en", "Origin": "https://money.tmx.com", "Referer": "https://money.tmx.com/"}
KEYS = ("ca", "us", "intl")
_last_call = {}


def _pace(host, seconds=0.6):
    wait = _last_call.get(host, 0) + seconds - time.time()
    if wait > 0:
        time.sleep(wait)
    _last_call[host] = time.time()


def _num(v, default=None):
    try:
        s = str(v).replace("$", "").replace("%", "").replace(",", "").strip()
        return float(s) if s not in ("", "N/A", "NA", "None") else default
    except (TypeError, ValueError):
        return default


def sector_of(name):
    return exposure.norm_sector(name) or "Not classified"


def parse_screener(data):
    """Nasdaq's screener rows into {symbol, name, last, percentChange, cap, sector, country}."""
    out = []
    for r in ((data or {}).get("data") or {}).get("rows") or []:
        if not isinstance(r, dict) or not r.get("symbol"):
            continue
        out.append({"symbol": str(r["symbol"]).strip(), "name": str(r.get("name") or "").strip(), "last": _num(r.get("lastsale")),
                    "percentChange": _num(r.get("pctchange")), "cap": _num(r.get("marketCap"), 0.0) or 0.0,
                    "sector": sector_of(r.get("sector")), "country": str(r.get("country") or "").strip()})
    return out


def us_rows(rows, n=TOP):
    """The n largest US companies by market cap, tiles sized by market cap."""
    picked = sorted([r for r in rows if r["country"] == "United States" and r["cap"] > 0], key=lambda r: -r["cap"])[:n]
    return [{"symbol": r["symbol"], "name": r["name"], "value": r["cap"], "percentChange": r["percentChange"], "sector": r["sector"], "country": r["country"]} for r in picked]


def intl_rows(rows, n=TOP):
    """The n largest companies listed in the US from outside the US and Canada, by market cap."""
    picked = sorted([r for r in rows if r["country"] not in ("United States", "Canada", "") and r["cap"] > 0], key=lambda r: -r["cap"])[:n]
    return [{"symbol": r["symbol"], "name": r["name"], "value": r["cap"], "percentChange": r["percentChange"], "sector": r["sector"], "country": r["country"]} for r in picked]


def parse_constituents(data):
    out = []
    for c in ((data or {}).get("data") or {}).get("constituents") or []:
        if not isinstance(c, dict) or not c.get("symbol"):
            continue
        out.append({"symbol": str(c["symbol"]).strip(), "name": str(c.get("longName") or c.get("shortName") or "").strip(),
                    "weight": _num(c.get("weight"), 0.0) or 0.0, "cap": _num(c.get("quotedMarketValue"), 0.0) or 0.0, "exchange": str(c.get("exchange") or "").strip()})
    return out


def parse_tile_quote(data):
    q = ((data or {}).get("data") or {}).get("getQuoteBySymbol") or {}
    if not isinstance(q, dict) or not q:
        return None
    return {"percentChange": _num(q.get("percentChange")), "sector": sector_of(q.get("sector")), "name": str(q.get("name") or "").strip()}


def fetch_screener(ssl_context=None):
    _pace("api.nasdaq.com")
    return parse_screener(json.loads(market._get_text(SCREENER_URL, ssl_context, headers=NASDAQ_HEADERS)))


def fetch_canada(ssl_context=None):
    """The S&P/TSX 60: its constituents by index weight, each quoted for the day's change and its sector."""
    _pace("app-money.tmx.com")
    cons = parse_constituents(market._post_json(market.TMX_URL, {"operationName": "getIndexConstituents", "variables": {"symbol": CANADA_INDEX}, "query": TMX_CONSTITUENTS_QUERY}, ssl_context, TMX_HEADERS))
    out = []
    for c in cons:
        q = None
        try:
            _pace("app-money.tmx.com")
            q = parse_tile_quote(market._post_json(market.TMX_URL, {"operationName": "getQuoteBySymbol", "variables": {"symbol": c["symbol"], "locale": "en"}, "query": TMX_TILE_QUERY}, ssl_context, TMX_HEADERS))
        except Exception as e:
            sys.stderr.write("bagholder universes: %s quote failed: %s\n" % (c["symbol"], e))
        out.append({"symbol": c["symbol"], "name": c["name"], "value": c["weight"] or c["cap"], "percentChange": q["percentChange"] if q else None,
                    "sector": q["sector"] if q else "Not classified", "country": "Canada"})
    return out


def refresh(ssl_context=None):
    """Read every universe; each answer replaces its rows. Returns the keys that answered."""
    done = []
    try:
        rows = fetch_screener(ssl_context)
        store.replace_universe("us", us_rows(rows))
        store.replace_universe("intl", intl_rows(rows))
        done += ["us", "intl"]
    except Exception as e:
        sys.stderr.write("bagholder universes: Nasdaq's screener failed: %s\n" % e)
    try:
        ca = fetch_canada(ssl_context)
        if ca:
            store.replace_universe("ca", ca)
            done.append("ca")
    except Exception as e:
        sys.stderr.write("bagholder universes: the S&P/TSX 60 failed: %s\n" % e)
    return done
