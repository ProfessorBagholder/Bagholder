"""Sector and country exposure of the book's holdings, for the Portfolio tab.

A share is classified by its listing's own record: TMX Money's quote carries a sector
and an industry for Canadian and US listings (Nasdaq's summary is the fallback for a
US one), and the country is the listing venue's. A fund is looked through: every
Canadian ETF publishes its holdings, so each issuer adapter has one job, the fund's
holdings with tickers and weights (or, where the issuer states them, the fund's own
sector and country breakdowns), and the holdings are classified here by the same
records as a share, a holding that is itself a fund being looked through in turn.
Nothing here asks Wealthsimple for anything. What no source covers is reported as
unclassified, never guessed.
"""
from __future__ import annotations

import csv
import io
import json
import re
import time
from datetime import datetime, timezone
from html.parser import HTMLParser

import market
import store

UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36"
HEADERS = {"User-Agent": UA, "Accept": "text/html,application/json;q=0.9,*/*;q=0.8", "Accept-Language": "en-CA,en;q=0.9"}
PACE_SEC = 0.6              # between two requests to the same issuer
MAX_DEPTH = 3               # a fund of funds of funds is as deep as the look-through goes
FRESH_DAYS = 7              # a breakdown older than this is fetched again
SHARE_KEY = "share:"        # cache keys for a classified listing: share:<TICKER>:<venue form>
FUND_KEY = "fund:"          # ... and for a fund reached through another fund's holdings


def _s(v):
    return "" if v is None else str(v)


def _num(v, default=None):
    try:
        s = _s(v).strip().replace(",", "").replace("%", "")
        if s.startswith("(") and s.endswith(")"):
            s = "-" + s[1:-1]
        return float(s)
    except (TypeError, ValueError):
        return default


def _now_iso():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


_last_call = {}


def _pace(host):
    """One request per PACE_SEC per host: issuers are read, never hammered."""
    last = _last_call.get(host, 0.0)
    wait = PACE_SEC - (time.time() - last)
    if wait > 0:
        time.sleep(wait)
    _last_call[host] = time.time()


def _get(url, headers=None):
    _pace(url.split("/")[2])
    return market._get_text(url, headers=dict(HEADERS, **(headers or {})))


def _post(url, payload, headers=None):
    _pace(url.split("/")[2])
    return market._post_json(url, payload, None, dict(HEADERS, **(headers or {})))


# --------------------------------------------------------------------------
# names: every source has its own words for the same sector and country
# --------------------------------------------------------------------------

SECTORS = ("Information Technology", "Financials", "Health Care", "Consumer Discretionary", "Consumer Staples", "Industrials",
           "Energy", "Materials", "Utilities", "Real Estate", "Communication Services")
_SECTOR_ALIAS = {
    "technology": "Information Technology", "information technology": "Information Technology", "tech": "Information Technology",
    "financial": "Financials", "financials": "Financials", "financial services": "Financials", "finance": "Financials", "banks": "Financials",
    "health care": "Health Care", "healthcare": "Health Care",
    "consumer discretionary": "Consumer Discretionary", "consumer cyclicals": "Consumer Discretionary", "consumer, cyclical": "Consumer Discretionary", "consumer cyclical": "Consumer Discretionary",
    "consumer staples": "Consumer Staples", "consumer non-cyclicals": "Consumer Staples", "consumer, non-cyclical": "Consumer Staples", "consumer non-cyclical": "Consumer Staples", "consumer defensive": "Consumer Staples",
    "industrials": "Industrials", "industrial": "Industrials",
    "energy": "Energy",
    "materials": "Materials", "basic materials": "Materials",
    "utilities": "Utilities",
    "real estate": "Real Estate", "realestate": "Real Estate",
    "communication services": "Communication Services", "communications": "Communication Services", "communication": "Communication Services", "media": "Communication Services", "telecommunications services": "Communication Services", "telecommunications": "Communication Services", "telecommunication services": "Communication Services",
    "bitcoin holding": "Digital assets", "digital assets": "Digital assets", "cryptocurrency": "Digital assets", "crypto": "Digital assets",
    "cash and/or derivatives": "", "cash": "", "other": "", "miscellaneous": "", "-": "", "n/a": "",
}


def norm_sector(name):
    """The sector under the name the Portfolio uses, '' for none (cash, other, blank)."""
    key = _s(name).strip().lower()
    if not key:
        return ""
    if key in _SECTOR_ALIAS:
        return _SECTOR_ALIAS[key]
    return _s(name).strip()


_COUNTRY_ALIAS = {
    "united states": "United States", "united states of america": "United States", "usa": "United States", "us": "United States", "u.s.": "United States", "u.s.a.": "United States",
    "canada": "Canada", "ca": "Canada", "can": "Canada",
    "united kingdom": "United Kingdom", "uk": "United Kingdom", "gb": "United Kingdom", "great britain": "United Kingdom", "britain": "United Kingdom",
    "korea": "South Korea", "korea, republic of": "South Korea", "republic of korea": "South Korea", "south korea": "South Korea",
    "taiwan, province of china": "Taiwan", "taiwan": "Taiwan", "hong kong sar": "Hong Kong", "hong kong": "Hong Kong",
    "russian federation": "Russia", "viet nam": "Vietnam", "czech republic": "Czechia",
    "broad": "", "global": "", "other": "", "-": "", "n/a": "", "cash": "",
}
# the venue a listing trades on says which country it is listed in
VENUE_COUNTRY = {
    "TSX": "Canada", "TSX-V": "Canada", "TSXV": "Canada", "CSE": "Canada", "CBOE CANADA": "Canada", "NEO": "Canada", "ALPHA EXCHANGE": "Canada",
    "TORONTO STOCK EXCHANGE": "Canada", "TSX VENTURE EXCHANGE": "Canada", "CANADIAN SECURITIES EXCHANGE": "Canada",
    "NYSE": "United States", "NASDAQ": "United States", "NYSE ARCA": "United States", "NYSE AMERICAN": "United States", "BATS": "United States", "AMEX": "United States", "ARCA": "United States",
    "NASDAQ GLOBAL SELECT": "United States", "NASDAQ GLOBAL MARKET": "United States", "NASDAQ CAPITAL MARKET": "United States", "NEW YORK STOCK EXCHANGE": "United States",
}
# Bloomberg's market codes, as issuers write tickers ("MSFT US EQUITY")
BLOOMBERG_COUNTRY = {"US": "United States", "UN": "United States", "UW": "United States", "UQ": "United States", "UA": "United States", "CN": "Canada", "CT": "Canada", "CV": "Canada",
                     "LN": "United Kingdom", "JP": "Japan", "JT": "Japan", "GR": "Germany", "GY": "Germany", "FP": "France", "AU": "Australia", "AT": "Australia", "HK": "Hong Kong",
                     "SW": "Switzerland", "SE": "Switzerland", "NA": "Netherlands", "SM": "Spain", "IM": "Italy", "KS": "South Korea", "TT": "Taiwan", "IN": "India", "IS": "India",
                     "BZ": "Brazil", "SS": "Sweden", "DC": "Denmark", "NO": "Norway", "FH": "Finland", "BB": "Belgium", "ID": "Ireland", "SP": "Singapore", "MM": "Mexico", "CH": "China", "C1": "China"}


def norm_country(name):
    key = _s(name).strip().lower()
    if not key:
        return ""
    if key in _COUNTRY_ALIAS:
        return _COUNTRY_ALIAS[key]
    return _s(name).strip()


def venue_country(exchange):
    return VENUE_COUNTRY.get(_s(exchange).strip().upper(), "")


# --------------------------------------------------------------------------
# a share: TMX Money's sector and industry, the venue's country
# --------------------------------------------------------------------------

TMX_SECTOR_QUERY = ("query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) "
                    "{ symbol name sector industry exchangeName } }")
NASDAQ_SUMMARY_URL = "https://api.nasdaq.com/api/quote/%s/summary?assetclass=stocks"


def _tmx_record(key):
    if not key:
        return {}
    try:
        _pace("app-money.tmx.com")
        d = market._post_json(market.TMX_URL, {"operationName": "getQuoteBySymbol", "variables": {"symbol": key, "locale": "en"}, "query": TMX_SECTOR_QUERY}, None, market._TMX_HEADERS)
    except Exception:
        return {}
    q = ((d or {}).get("data") or {}).get("getQuoteBySymbol") or {}
    return q if (q.get("sector") or q.get("industry") or q.get("name")) else {}


def _nasdaq_summary(symbol):
    try:
        raw = _get(NASDAQ_SUMMARY_URL % symbol, {"Accept": "application/json, text/plain, */*"})
        d = json.loads(raw)
    except Exception:
        return {}
    s = ((d or {}).get("data") or {}).get("summaryData") or {}
    return {"sector": _s((s.get("Sector") or {}).get("value")), "industry": _s((s.get("Industry") or {}).get("value"))}


def classify_share(symbol, exchange="", currency=""):
    """{sector, industry, country, source} for one listing, from TMX's record (any
    venue TMX carries), Nasdaq's for a US listing TMX has no sector for; the country
    is the venue's. Blank fields mean the record has none."""
    sym = market.tmx_symbol(symbol)
    country = venue_country(exchange)
    out = {"sector": "", "industry": "", "country": country, "source": ""}
    key = market.tmx_quote_symbol(symbol, exchange, currency)
    if key is None and sym and " " not in sym:
        key = sym + (":US" if _s(currency).upper() == "USD" else "")
    if key:
        rec, _form = market.tmx_lookup(key, _tmx_record)
        if rec and not country and re.search(r"\bCDR\b", _s(rec.get("name"))) and not key.endswith(":US"):
            # a bare ticker with no venue given answered with the Canadian depositary receipt of a
            # US company; the company itself is the US listing
            us = _tmx_record(market.tmx_bare(key) + ":US")
            if us:
                rec = us
        if rec:
            out["sector"] = norm_sector(rec.get("sector"))
            out["industry"] = _s(rec.get("industry")).strip()
            out["country"] = country or venue_country(rec.get("exchangeName"))
            out["source"] = "TMX Money"
    if not out["sector"] and (country == "United States" or _s(currency).upper() == "USD") and sym and " " not in sym:
        nq = _nasdaq_summary(sym)
        if nq.get("sector"):
            out["sector"] = norm_sector(nq["sector"])
            out["industry"] = out["industry"] or nq.get("industry", "")
            out["country"] = out["country"] or "United States"
            out["source"] = out["source"] or "Nasdaq"
    return out


# --------------------------------------------------------------------------
# HTML tables, the one shape most issuer pages share
# --------------------------------------------------------------------------

class _Tables(HTMLParser):
    """Every <table> on a page as rows of cell texts."""

    def __init__(self):
        super().__init__()
        self.tables, self._table, self._row, self._cell = [], None, None, None

    def handle_starttag(self, tag, attrs):
        if tag == "table":
            self._table = []
        elif tag == "tr" and self._table is not None:
            self._row = []
        elif tag in ("td", "th") and self._row is not None:
            self._cell = []
        elif tag == "br" and self._cell is not None:
            self._cell.append(" ")

    def handle_endtag(self, tag):
        if tag in ("td", "th") and self._cell is not None and self._row is not None:
            self._row.append(re.sub(r"\s+", " ", "".join(self._cell)).strip())
            self._cell = None
        elif tag == "tr" and self._row is not None and self._table is not None:
            if self._row:
                self._table.append(self._row)
            self._row = None
        elif tag == "table" and self._table is not None:
            self.tables.append(self._table)
            self._table = None

    def handle_data(self, data):
        if self._cell is not None:
            self._cell.append(data)


def html_tables(html):
    p = _Tables()
    try:
        p.feed(html)
    except Exception:
        pass
    return p.tables


def _header_index(header, *names):
    """The column index whose header contains one of the names, else -1."""
    low = [h.lower() for h in header]
    for n in names:
        for i, h in enumerate(low):
            if n in h:
                return i
    return -1


# --------------------------------------------------------------------------
# issuers: each adapter answers with the fund's own breakdowns and/or its holdings
#   {"sectors": {name: percent}, "countries": {name: percent}, "holdings": [...], "source": url, "asOf": date}
#   a holding: {"ticker", "exchange", "currency", "name", "weight" (percent), "sector", "country", "fund": bool}
# --------------------------------------------------------------------------

_ISSUERS = (
    ("vanguard", r"^vanguard\b"), ("ishares", r"^ishares\b"), ("harvest", r"^harvest\b"), ("ninepoint", r"^ninepoint\b"), ("evolve", r"^evolve\b"),
    ("bmo", r"^bmo\b"), ("globalx", r"^(global x|horizons)\b"),
)


def issuer_of(name, symbol=""):
    n = _s(name).strip().lower()
    for key, pat in _ISSUERS:
        if re.search(pat, n):
            return key
    return ""


def is_fund(name, symbol=""):
    n = _s(name)
    return bool(re.search(r"\b(ETF|Index|Fund|Portfolio|Trust)\b", n, re.I)) or bool(issuer_of(n))


# --- Vanguard Canada: its GraphQL states the fund's sector and country breakdowns ---
VANGUARD_GQL = "https://www.vanguard.ca/gpx/graphql"
VANGUARD_HEADERS = {"Content-Type": "application/json", "X-Consumer-ID": "ca0", "apollographql-client-name": "gpx", "Origin": "https://www.vanguard.ca", "Referer": "https://www.vanguard.ca/en/product"}
# every Canadian Vanguard portfolio id the product list names (2026-09); the finder maps them to tickers
VANGUARD_PORT_IDS = ["1811", "1817", "1936", "9561", "9554", "9559", "9560", "9569", "9570", "9558", "9555", "9550", "9549", "9742", "9556", "9548", "9828", "9835", "9795", "9563", "9562",
                     "9566", "9564", "9551", "9567", "9870", "9841", "9552", "9553", "9565", "9568", "9691", "9577", "9578", "9579", "9692", "9557", "9864", "9865", "9867", "9896"]
_vanguard_map = {}


def _vanguard_port_id(symbol):
    if not _vanguard_map:
        q = {"operationName": "FundFinderFunds", "variables": {"portIds": VANGUARD_PORT_IDS},
             "query": "query FundFinderFunds($portIds: [String!]!) { funds(portIds: $portIds) { portId profile { fundFullName listings { identifiers(altIds: [\"Ticker - Canada\", \"Ticker\"]) { altId altIdValue } } } } }"}
        d = _post(VANGUARD_GQL, q, VANGUARD_HEADERS)
        for f in ((d or {}).get("data") or {}).get("funds") or []:
            p = f.get("profile") or {}
            for l in p.get("listings") or []:
                for i in l.get("identifiers") or []:
                    if i.get("altIdValue"):
                        _vanguard_map.setdefault(_s(i["altIdValue"]).upper(), _s(f.get("portId") or p.get("portId")))
    return _vanguard_map.get(market.tmx_symbol(symbol), "")


def vanguard_ca(symbol, name, exchange):
    pid = _vanguard_port_id(symbol)
    if not pid:
        return None
    sec = _post(VANGUARD_GQL, {"operationName": "getSectorDiversification", "variables": {"portIds": [pid]},
                               "query": "query getSectorDiversification($portIds: [String!]!) { funds(portIds: $portIds) { sectorDiversification { sectorName fundPercent date } } }"}, VANGUARD_HEADERS)
    mkt = _post(VANGUARD_GQL, {"operationName": "MarketAllocationGqlQuery", "variables": {"portIds": [pid]},
                               "query": "query MarketAllocationGqlQuery($portIds: [String!]!) { funds(portIds: $portIds) { marketAllocation { countryName fundMktPercent date } } }"}, VANGUARD_HEADERS)
    srows = (((sec or {}).get("data") or {}).get("funds") or [{}])[0].get("sectorDiversification") or []
    crows = (((mkt or {}).get("data") or {}).get("funds") or [{}])[0].get("marketAllocation") or []
    sectors, countries = {}, {}
    for r in srows:
        n, w = norm_sector(r.get("sectorName")), _num(r.get("fundPercent"), 0.0)
        if n and w > 0:
            sectors[n] = sectors.get(n, 0.0) + w
    for r in crows:
        n, w = norm_country(r.get("countryName")), _num(r.get("fundMktPercent"), 0.0)
        if n and w > 0:
            countries[n] = countries.get(n, 0.0) + w
    if not sectors and not countries:
        return None
    as_of = _s((srows or crows or [{}])[0].get("date"))
    return {"sectors": sectors, "countries": countries, "holdings": [], "source": "Vanguard Canada", "asOf": as_of}


# --- iShares Canada: the screener names every fund's page; the page's holdings CSV has sector and location per holding ---
ISHARES_SCREENER = "https://www.blackrock.com/ca/investors/en/product-screener/product-screener-v3.1.jsn?dcrPath=/templatedata/config/product-screener-v3/data/en/ca-one/product-screener-backend-config&siteEntryPassthrough=true"
ISHARES_HOLDINGS = "https://www.blackrock.com%s/1464253357814.ajax?fileType=csv&fileName=holdings&dataType=fund"
_ishares_map = {}


def _ishares_page(symbol):
    if not _ishares_map:
        raw = _get(ISHARES_SCREENER, {"Accept": "application/json, text/plain, */*"})
        d = json.loads(raw.lstrip("﻿"))
        for rec in (d.values() if isinstance(d, dict) else []):
            if isinstance(rec, dict) and rec.get("localExchangeTicker") and rec.get("productPageUrl"):
                _ishares_map[_s(rec["localExchangeTicker"]).upper()] = _s(rec["productPageUrl"])
    return _ishares_map.get(market.tmx_symbol(symbol), "")


def parse_ishares_csv(text):
    """The holdings CSV: a few preamble lines, then a header row starting with Ticker.
    Returns (holdings, as_of)."""
    lines = text.lstrip("﻿").splitlines()
    as_of = ""
    start = -1
    for i, line in enumerate(lines):
        if line.startswith("Fund Holdings as of"):
            parts = next(csv.reader([line]))
            as_of = parts[1] if len(parts) > 1 else ""
        if line.startswith("Ticker,") or line.startswith('"Ticker"'):
            start = i
            break
    if start < 0:
        return [], as_of
    rows = list(csv.reader(lines[start:]))
    header = [h.strip() for h in rows[0]]
    it, iname, isec, icls, iw, iloc, iex, iccy = (_header_index(header, "ticker"), _header_index(header, "name"), _header_index(header, "sector"), _header_index(header, "asset class"),
                                                 _header_index(header, "weight"), _header_index(header, "location"), _header_index(header, "exchange"), _header_index(header, "currency"))
    out = []
    for r in rows[1:]:
        if len(r) <= max(it, iw) or not r[it].strip():
            continue
        cls = r[icls].strip().lower() if icls >= 0 and icls < len(r) else ""
        w = _num(r[iw], 0.0) if iw >= 0 else 0.0
        if w <= 0 or cls in ("cash", "money market", "futures", "derivatives", "forwards", "fx"):
            continue
        nm = r[iname].strip() if iname >= 0 else ""
        out.append({"ticker": r[it].strip(), "name": nm, "weight": w, "sector": norm_sector(r[isec]) if isec >= 0 else "",
                    "country": norm_country(r[iloc]) if iloc >= 0 else "", "exchange": r[iex].strip() if iex >= 0 else "", "currency": r[iccy].strip() if iccy >= 0 else "",
                    "fund": "ISHARES" in nm.upper() or is_fund(nm)})
    return out, as_of


def ishares_ca(symbol, name, exchange):
    page = _ishares_page(symbol)
    if not page:
        return None
    holdings, as_of = parse_ishares_csv(_get(ISHARES_HOLDINGS % page, {"Accept": "text/csv,*/*"}))
    if not holdings:
        return None
    for h in holdings:
        if h["fund"]:
            h["sector"] = ""          # a fund's row says Other or the top holding's sector; the fund is looked through instead
    return {"sectors": {}, "countries": {}, "holdings": holdings, "source": "iShares Canada", "asOf": as_of}


# --- Harvest: the product page's tables; a single-stock fund names its reference asset ---
HARVEST_PAGE = "https://harvestportfolios.com/etf/%s/"


def parse_harvest_tables(tables):
    """Holdings from a Harvest page's tables, whichever shape the fund's page uses:
    Name | Ticker | Weight | Sector | Country; HOLDINGS name | weight; a Reference Asset row."""
    holdings, ref = [], ""
    for t in tables:
        if not t:
            continue
        header = t[0]
        for row in t:
            if len(row) >= 2 and row[0].strip().lower().startswith("reference asset"):
                ref = row[1].strip()
        it, iw = _header_index(header, "ticker"), _header_index(header, "weight")
        iname, isec, ictry = _header_index(header, "name"), _header_index(header, "sector"), _header_index(header, "country")
        if it >= 0 and iw >= 0 and iname >= 0:
            for r in t[1:]:
                if len(r) <= max(it, iw, iname):
                    continue
                w = _num(r[iw], 0.0)
                nm, tk = r[iname].strip(), r[it].strip()
                if w <= 0 or not nm or re.search(r"written options|cash and other|cash & other", nm, re.I):
                    continue
                parts = tk.split()
                sym, code = (parts[0], parts[1]) if len(parts) >= 2 else (tk, "")
                holdings.append({"ticker": sym, "name": nm, "weight": w, "sector": norm_sector(r[isec]) if isec >= 0 and isec < len(r) else "",
                                 "country": (norm_country(r[ictry]) if ictry >= 0 and ictry < len(r) else "") or BLOOMBERG_COUNTRY.get(code.upper(), ""),
                                 "exchange": "", "currency": "", "fund": is_fund(nm)})
            continue
        if header and re.match(r"^holdings?\b", header[0].strip(), re.I):
            for r in t[1:]:
                if len(r) < 2:
                    continue
                nm, w = r[0].strip(), _num(r[1], 0.0)
                if w <= 0 or not nm or re.search(r"written options|cash and other|cash & other", nm, re.I):
                    continue
                holdings.append({"ticker": "", "name": nm, "weight": w, "sector": "", "country": "", "exchange": "", "currency": "", "fund": is_fund(nm)})
    return holdings, ref


def harvest(symbol, name, exchange):
    html = _get(HARVEST_PAGE % market.tmx_symbol(symbol).lower())
    holdings, ref = parse_harvest_tables(html_tables(html))
    if ref and re.fullmatch(r"[A-Z0-9][A-Z0-9.:-]{0,9}", ref) and not any(h["ticker"] for h in holdings):
        # a single-stock fund names its reference asset as a ticker: that is the whole exposure
        # (a fund of funds writes words there, and its HOLDINGS table is what counts)
        holdings = [{"ticker": ref, "name": ref, "weight": 100.0, "sector": "", "country": "", "exchange": "", "currency": "", "fund": False}]
    if not holdings:
        return None
    return {"sectors": {}, "countries": {}, "holdings": holdings, "source": "Harvest ETFs", "asOf": ""}


# --- Ninepoint: each HighShares fund's page names its underlying stock ---
NINEPOINT_LIST = "https://www.ninepoint.com/landing-pages/ninepoint-highshares-etfs/"
NINEPOINT_BASE = "https://www.ninepoint.com"
_ninepoint_pages = {}


def parse_ninepoint_page(html):
    """(ticker, underlying ticker, underlying exchange) from a Ninepoint fund page:
    `Ticker | CCHI:TSX` and `Underlying Stock** | Cameco Corp. (CCO:TSX)`."""
    text = re.sub(r"<[^>]+>", " ", html)
    text = re.sub(r"\s+", " ", text)
    m = re.search(r"Ticker\s*\*?\*?\s*([A-Z0-9.]{1,8}):([A-Z]{2,6})\b", text)
    u = re.search(r"Underlying Stock\s*\*?\*?\s*(.*?)\(([A-Z0-9.]{1,8}):([A-Z]{2,6})\)", text)
    return (m.group(1) if m else "", u.group(2) if u else "", u.group(3) if u else "")


def _ninepoint_slugs():
    html = _get(NINEPOINT_LIST)
    return sorted(set(re.findall(r'href="(?:https://www\.ninepoint\.com)?(/funds/[a-z0-9-]+/)"', html)))


def ninepoint(symbol, name, exchange):
    sym = market.tmx_symbol(symbol)
    if sym not in _ninepoint_pages:
        # the landing page lists every HighShares fund; a page is read once to learn its ticker
        for slug in _ninepoint_slugs():
            if sym in _ninepoint_pages:
                break
            if slug in _ninepoint_pages.values():
                continue
            try:
                t, under, ex = parse_ninepoint_page(_get(NINEPOINT_BASE + slug))
            except Exception:
                continue
            if t:
                _ninepoint_pages[t] = slug
    slug = _ninepoint_pages.get(sym)
    if not slug:
        return None
    t, under, ex = parse_ninepoint_page(_get(NINEPOINT_BASE + slug))
    if not under:
        return None
    return {"sectors": {}, "countries": {}, "holdings": [{"ticker": under, "name": under, "weight": 100.0, "sector": "", "country": "", "exchange": ex, "currency": "", "fund": False}],
            "source": "Ninepoint", "asOf": ""}


# --- Evolve: the product page embeds its holdings and its sector breakdown as script data ---
EVOLVE_PAGE = "https://evolveetfs.com/product/%s/"


def parse_evolve_page(html):
    """(sectors, holdings) from the page's `portfolioBreakdownData` and `holdingsData`."""
    sectors, holdings = {}, []
    m = re.search(r"var portfolioBreakdownData\s*=\s*(\{.*?\});\s*\n", html, re.S)
    if m:
        try:
            for r in ((json.loads(m.group(1)).get("data") or {}).get("sector") or []):
                n, w = norm_sector(r.get("name")), _num(r.get("weight"), 0.0)
                if n and w > 0:
                    sectors[n] = sectors.get(n, 0.0) + w
        except ValueError:
            pass
    m = re.search(r"var holdingsData\s*=\s*(\{.*?\});\s*\n", html, re.S)
    if m:
        try:
            rows = json.loads(m.group(1)).get("data") or []
        except ValueError:
            rows = []
        for r in rows:
            tk = _s(r.get("ticker")).strip()
            parts = tk.split()
            sym, code = (parts[0], parts[1]) if len(parts) >= 2 else (tk, "")
            w = _num(r.get("weight_percent"), 0.0)
            nm = _s(r.get("security_name")).strip()
            if w <= 0 or not sym:
                continue
            ctry = norm_country(r.get("country"))
            if ctry and (len(ctry) <= 5 and ctry.upper() == ctry):
                ctry = ""     # a fund-of-funds page writes the sub-fund's ticker here, not a country
            holdings.append({"ticker": sym, "name": nm, "weight": w, "sector": norm_sector(r.get("gics_sector")), "country": ctry or BLOOMBERG_COUNTRY.get(code.upper(), ""),
                             "exchange": "", "currency": "", "fund": is_fund(nm)})
    return sectors, holdings


def evolve(symbol, name, exchange):
    sectors, holdings = parse_evolve_page(_get(EVOLVE_PAGE % market.tmx_symbol(symbol).lower()))
    if not sectors and not holdings:
        return None
    return {"sectors": sectors, "countries": {}, "holdings": holdings, "source": "Evolve ETFs", "asOf": ""}


# --- any other family: Yahoo's fund profile, which names every fund's sector weightings and top holdings ---
YAHOO_CRUMB = "https://query2.finance.yahoo.com/v1/test/getcrumb"
YAHOO_SUMMARY = "https://query2.finance.yahoo.com/v10/finance/quoteSummary/%s?modules=topHoldings&crumb=%s"
YAHOO_SUFFIX = {"TSX": ".TO", "TSX-V": ".V", "TSXV": ".V", "CSE": ".CN", "CBOE CANADA": ".NE", "NEO": ".NE"}
_yahoo = {"cookie": "", "crumb": ""}


def _yahoo_session():
    """Yahoo answers its quote summary only with a session cookie and the crumb it hands out for it."""
    if _yahoo["crumb"]:
        return _yahoo
    from urllib.request import Request, urlopen
    _pace("fc.yahoo.com")
    req = Request("https://fc.yahoo.com", headers={"User-Agent": UA})
    try:
        with urlopen(req, timeout=market.TIMEOUT_SEC, context=market.default_ssl_context()) as resp:
            cookies = [v.split(";")[0] for k, v in resp.headers.items() if k.lower() == "set-cookie"]
    except Exception as e:
        cookies = [v.split(";")[0] for v in (getattr(e, "headers", None) or {}).get_all("set-cookie", [])] if hasattr(e, "headers") else []
    _yahoo["cookie"] = "; ".join(cookies)
    _yahoo["crumb"] = _get(YAHOO_CRUMB, {"Cookie": _yahoo["cookie"]}).strip()
    return _yahoo


def yahoo_symbol(symbol, exchange):
    return market.tmx_symbol(symbol) + YAHOO_SUFFIX.get(_s(exchange).strip().upper(), "")


def parse_yahoo_summary(data):
    """(sectors, holdings) from Yahoo's topHoldings module."""
    res = (((data or {}).get("quoteSummary") or {}).get("result") or [{}])[0]
    th = res.get("topHoldings") or {}
    sectors = {}
    for entry in th.get("sectorWeightings") or []:
        for k, v in (entry or {}).items():
            w = _num((v or {}).get("raw"), 0.0) if isinstance(v, dict) else _num(v, 0.0)
            n = norm_sector(k.replace("_", " "))
            if n and w > 0:
                sectors[n] = round(sectors.get(n, 0.0) + w * 100.0, 4)
    holdings = []
    for h in th.get("holdings") or []:
        sym = _s(h.get("symbol")).strip()
        w = _num((h.get("holdingPercent") or {}).get("raw"), 0.0) if isinstance(h.get("holdingPercent"), dict) else _num(h.get("holdingPercent"), 0.0)
        if sym and w > 0:
            ex = ""
            for suf, venue in ((".TO", "TSX"), (".V", "TSX-V"), (".CN", "CSE"), (".NE", "CBOE CANADA")):
                if sym.upper().endswith(suf):
                    ex = venue
            holdings.append({"ticker": sym, "name": _s(h.get("holdingName")), "weight": round(w * 100.0, 4), "sector": "", "country": "", "exchange": ex,
                             "currency": "CAD" if ex else "", "fund": is_fund(_s(h.get("holdingName")))})
    return sectors, holdings


def yahoo_fund(symbol, name, exchange):
    sess = _yahoo_session()
    raw = _get(YAHOO_SUMMARY % (yahoo_symbol(symbol, exchange), sess["crumb"]), {"Cookie": sess["cookie"], "Accept": "application/json"})
    sectors, holdings = parse_yahoo_summary(json.loads(raw))
    if not sectors and not holdings:
        return None
    return {"sectors": sectors, "countries": {}, "holdings": holdings, "source": "Yahoo Finance", "asOf": ""}


ADAPTERS = {"vanguard": vanguard_ca, "ishares": ishares_ca, "harvest": harvest, "ninepoint": ninepoint, "evolve": evolve}
FALLBACK = yahoo_fund


# --------------------------------------------------------------------------
# the look-through
# --------------------------------------------------------------------------

def resolve_name(name):
    """A holding named without a ticker: the directories' first match on the name."""
    import bagholder
    clean = re.sub(r"\b(inc|corp|corporation|ltd|limited|plc|co|class [a-z]|common shares?|common stock|the)\b\.?", " ", _s(name), flags=re.I)
    clean = re.sub(r"[^A-Za-z0-9 &.-]", " ", clean)
    clean = re.sub(r"\s+", " ", clean).strip()
    if not clean:
        return None
    r = bagholder.symbol_search(clean[:40])
    for m in r.get("matches") or []:
        return m
    return None


def _cache_get(key):
    rec = store.exposure_record(key)
    if not rec:
        return None
    try:
        age = (datetime.now(timezone.utc) - datetime.strptime(rec.get("fetchedAt") or "", "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)).days
    except ValueError:
        return None
    return rec if age < FRESH_DAYS else None


def share_exposure(symbol, exchange="", currency=""):
    """A classified share as an exposure record, cached by ticker and venue form."""
    key = SHARE_KEY + market.tmx_symbol(symbol) + ":" + (market.tmx_form(exchange, currency) or "")
    hit = _cache_get(key)
    if hit:
        return hit
    c = classify_share(symbol, exchange, currency)
    rec = {"sectors": {c["sector"]: 1.0} if c["sector"] else {}, "countries": {c["country"]: 1.0} if c["country"] else {},
           "coverage": 1.0 if c["sector"] or c["country"] else 0.0, "source": c["source"], "asOf": "", "industry": c["industry"]}
    store.replace_exposure(key, rec)
    return rec


def lookthrough(holdings, depth=0, seen=None):
    """Holdings into {sectors, countries, coverage}: weights over the positive rows,
    each row classified as given, by its ticker, by its name, or by looking a fund
    through. Fractions, summing to at most one; the rest is unclassified."""
    seen = seen or set()
    rows = [h for h in holdings if _num(h.get("weight"), 0.0) > 0]
    total = sum(_num(h.get("weight"), 0.0) for h in rows)
    sectors, countries, covered = {}, {}, 0.0
    if total <= 0:
        return {"sectors": {}, "countries": {}, "coverage": 0.0}
    for h in rows:
        w = _num(h.get("weight"), 0.0) / total
        sec, ctry = h.get("sector") or "", h.get("country") or ""
        tk, ex, ccy = h.get("ticker") or "", h.get("exchange") or "", h.get("currency") or ""
        if sec and ctry:
            # the issuer states both: nothing to look up
            sectors[sec] = sectors.get(sec, 0.0) + w
            countries[ctry] = countries.get(ctry, 0.0) + w
            covered += w
            continue
        if not tk and h.get("name"):
            # a holding named without a ticker: the directories give it, a fund's name included
            m = resolve_name(h["name"])
            if m:
                tk, ex, ccy = m.get("symbol", ""), m.get("exchange", ""), m.get("currency", "")
        sub = None
        if h.get("fund") and depth < MAX_DEPTH and (tk or h.get("name")):
            sub = fund_exposure(tk, h.get("name") or "", ex, depth + 1, seen)
        if sub and (sub.get("sectors") or sub.get("countries")):
            for n, f in (sub.get("sectors") or {}).items():
                sectors[n] = sectors.get(n, 0.0) + w * f
            for n, f in (sub.get("countries") or {}).items():
                countries[n] = countries.get(n, 0.0) + w * f
            covered += w * (sub.get("coverage") or 0.0)
            continue
        if tk:
            if not ex and ctry == "United States":
                ccy = ccy or "USD"
            c = share_exposure(tk, ex, ccy)
            sec = sec or next(iter(c.get("sectors") or {}), "")
            ctry = ctry or next(iter(c.get("countries") or {}), "")
        if sec:
            sectors[sec] = sectors.get(sec, 0.0) + w
        if ctry:
            countries[ctry] = countries.get(ctry, 0.0) + w
        if sec or ctry:
            covered += w
    return {"sectors": sectors, "countries": countries, "coverage": min(1.0, covered)}


def fund_exposure(symbol, name, exchange="", depth=0, seen=None):
    """A fund's {sectors, countries, coverage, source, asOf}, through its issuer's
    adapter, cached by ticker; None when no adapter covers its family or the source
    answered nothing."""
    seen = seen if seen is not None else set()
    key = FUND_KEY + market.tmx_symbol(symbol or name)
    if key in seen:
        return None
    seen.add(key)
    hit = _cache_get(key)
    if hit:
        return hit
    family = issuer_of(name, symbol)
    adapter = ADAPTERS.get(family)
    data = None
    if adapter:
        try:
            data = adapter(symbol, name, exchange)
        except Exception as e:
            market.note_source(family, False, e)
    if not data:
        # a family with no adapter, or one whose page answered nothing: the generic record
        try:
            data = FALLBACK(symbol, name, exchange)
        except Exception as e:
            market.note_source("yahoo", False, e)
    if not data:
        return None
    sectors = {n: w / 100.0 for n, w in (data.get("sectors") or {}).items()}
    countries = {n: w / 100.0 for n, w in (data.get("countries") or {}).items()}
    coverage = 1.0 if sectors or countries else 0.0
    if data.get("holdings") and (not sectors or not countries):
        agg = lookthrough(data["holdings"], depth, seen)
        sectors = sectors or agg["sectors"]
        countries = countries or agg["countries"]
        coverage = max(coverage, agg["coverage"]) if (sectors and countries) else agg["coverage"]
    tot_s, tot_c = sum(sectors.values()), sum(countries.values())
    if tot_s > 1.0001:
        sectors = {n: w / tot_s for n, w in sectors.items()}
    if tot_c > 1.0001:
        countries = {n: w / tot_c for n, w in countries.items()}
    rec = {"sectors": sectors, "countries": countries, "coverage": coverage, "source": data.get("source") or family, "asOf": data.get("asOf") or ""}
    store.replace_exposure(key, rec)
    return rec


def refresh_security(sec):
    """The exposure record for one of the book's securities, stored under its id:
    a fund looked through, a share classified. Returns the record (empty when no
    source covers it), never raises."""
    sid = _s(sec.get("id"))
    symbol, name, exchange, currency = _s(sec.get("symbol")), _s(sec.get("name")), _s(sec.get("primaryExchange")), _s(sec.get("currency"))
    rec = None
    try:
        if is_fund(name, symbol):
            # a fund no source covers is unclassified: its venue says nothing about what it holds
            rec = fund_exposure(symbol, name, exchange) or {"sectors": {}, "countries": {}, "coverage": 0.0, "source": "", "asOf": ""}
        else:
            rec = share_exposure(symbol, exchange, currency)
    except Exception as e:
        rec = {"sectors": {}, "countries": {}, "coverage": 0.0, "source": "", "asOf": "", "error": str(e) or e.__class__.__name__}
    store.replace_exposure(sid, rec)
    return rec


def stale(security_ids):
    """The ids among these whose record is missing or older than FRESH_DAYS."""
    out = []
    for sid in security_ids:
        if not _cache_get(_s(sid)):
            out.append(sid)
    return out
