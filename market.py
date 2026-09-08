"""Market data the derived model needs but Wealthsimple does not provide.

- USD/CAD daily average rate from the Bank of Canada Valet API (FXUSDCAD).
- S&P 500 index closes from FRED (SP500), with Stooq (^spx) as a fallback.

Both are persisted in SQLite (store.fx_rates / store.benchmark_prices) and
refreshed incrementally: only days after the newest stored date are fetched.
Every public function swallows network errors and returns what is stored.
"""

from __future__ import annotations

import gzip
import json
import os
import re
import sys
import ssl
import threading
from concurrent.futures import ThreadPoolExecutor
from datetime import date, datetime, timedelta, timezone
from zoneinfo import ZoneInfo
from urllib.request import Request, urlopen

import store

BOC_URL = "https://www.bankofcanada.ca/valet/observations/FXUSDCAD/json"
FRED_URL = "https://fred.stlouisfed.org/graph/fredgraph.csv?id=SP500"
STOOQ_URL = "https://stooq.com/q/d/l/?s=^spx&i=d"
TMX_URL = "https://app-money.tmx.com/graphql"
TMX_QUOTE_QUERY = (
    "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) "
    "{ symbol name exchangeName price priceChange percentChange prevClose currency dividendFrequency dividendYield dividendAmount exDividendDate } }"
)
QUOTE_REFRESH_MINUTES = 1
MARKET_CHECK_MINUTES = 60
US_EXCHANGES = ("NASDAQ", "NYSE", "NYSE AMERICAN", "NYSE ARCA", "BATS", "AMEX", "ARCA", "CBOE", "IEX")
TMX_DIVIDENDS_QUERY = (
    "query getDividendsForSymbol($symbol: String!, $page: Int, $batch: Int) { dividends: getDividendsForSymbol("
    "symbol: $symbol, page: $page, batch: $batch) { dividends { exDate payableDate amount currency } } }"
)
TMX_BATCH = 24
QUOTE_STALE_HOURS = 20
COINBASE_URL = "https://api.coinbase.com/v2/prices/%s/spot"
CBOE_CA_URL = "https://www-api.cboe.com/ca/equities/securities-1/%s/quote/"
CBOE_OPTIONS_URL = "https://cdn.cboe.com/api/global/delayed_quotes/options/%s.json"
CBOE_CANADA_EXCHANGES = ("CBOE CANADA", "NEO")
TMX_HISTORY_QUERY = (
    "query getTimeSeriesData($symbol: String!, $freq: String, $interval: Int, $start: String, $end: String) "
    "{ getTimeSeriesData(symbol: $symbol, freq: $freq, interval: $interval, start: $start, end: $end) { dateTime open high low close volume } }"
)
CBOE_CA_HISTORY_URL = "https://www-api.cboe.com/ca/equities/securities-1/%s/trading-activity-historical/"
COINBASE_EXCHANGE_PRODUCT_URL = "https://api.exchange.coinbase.com/products/%s"
COINBASE_CANDLES_URL = "https://api.exchange.coinbase.com/products/%s/candles?granularity=%d&start=%s&end=%s"
COINBASE_CANDLE_LIMIT = 300
COINBASE_EXCHANGE_START = "2015-01-01"
YAHOO_CHART_URL = "https://query1.finance.yahoo.com/v8/finance/chart/%s?period1=%d&period2=%d&interval=%s"
YAHOO_SUFFIX = {"TSX": ".TO", "TSX-V": ".V", "TSXV": ".V", "CSE": ".CN", "CBOE CANADA": ".NE", "NEO": ".NE"}
YAHOO_FORMS = {"CAD": (".TO", ".V", ".CN", ".NE"), "USD": ("",)}
YAHOO_INTRADAY_DAYS = 729
YAHOO_MIN_INTERVAL_SEC = 2.0    # Yahoo rate-limits bursts: one request at a time, well spaced
# Yahoo refuses the app's usual Safari User-Agent and CSV-first Accept header with
# 429 while answering a plain browser signature at once; it gets its own.
YAHOO_HEADERS = {"User-Agent": "Mozilla/5.0", "Accept": "application/json"}
YAHOO_BACKOFF_SEC = 600         # after a 429, leave Yahoo alone for this long
SOURCE_INTRADAY_DAYS = {"tmx": 365, "yahoo": YAHOO_INTRADAY_DAYS}   # coinbase: full history
TMX_CHART_QUERY = (
    "query getCompanyChart($symbol: String!, $from: String!, $to: String!) "
    "{ intraday: getChartDataBySymbol(symbol: $symbol, fromDate: $from, toDate: $to) { dateTime open high low close volume } }"
)
TMX_INTRADAY_DAYS = 365
SESSION_OPEN_MINUTES = 9 * 60 + 30
TIMEFRAMES = ("1h", "4h", "1d", "1w", "1M")
INTRADAY_SECONDS = {"1h": 3600, "4h": 14400}
HISTORY_STALE_HOURS = 20
RECORD_STALE_HOURS = QUOTE_STALE_HOURS
MARKET_ATTEMPT_HOURS = 6
CANADIAN_EXCHANGES = ("TSX", "TSX-V", "TSXV", "CSE", "CBOE CANADA", "NEO", "ALPHA EXCHANGE")
FX_START = "2016-01-01"
TIMEOUT_SEC = 30
STALE_DAYS = 4
UA = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
    "AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
)

_lock = threading.Lock()
_refreshing = False
_SSL_CTX = None


def default_ssl_context():
    """Same CA lookup as bagholder._ssl_context: certifi, then system bundles."""
    global _SSL_CTX
    if _SSL_CTX is not None:
        return _SSL_CTX
    ca_files = []
    try:
        import certifi

        ca_files.append(certifi.where())
    except Exception:
        pass
    ca_files.extend(
        (
            "/etc/ssl/cert.pem",
            "/etc/ssl/certs/ca-certificates.crt",
            "/opt/homebrew/etc/openssl@3/cert.pem",
            "/usr/local/etc/openssl@3/cert.pem",
            "/opt/homebrew/etc/openssl@1.1/cert.pem",
        )
    )
    for path in ca_files:
        if path and os.path.isfile(path):
            try:
                _SSL_CTX = ssl.create_default_context(cafile=path)
                return _SSL_CTX
            except Exception:
                continue
    _SSL_CTX = ssl.create_default_context()
    return _SSL_CTX


def _today():
    return datetime.now(timezone.utc).date()


def _get_text(url, ssl_context=None, headers=None):
    req = Request(url, headers=headers or {"User-Agent": UA, "Accept": "text/csv,application/json,*/*;q=0.8"})
    ctx = ssl_context or default_ssl_context()
    with urlopen(req, timeout=TIMEOUT_SEC, context=ctx) as resp:
        raw = resp.read()
    if raw[:2] == b"\x1f\x8b":
        try:
            raw = gzip.decompress(raw)
        except OSError:
            pass
    return raw.decode("utf-8", "replace")


def parse_boc_json(text):
    """{"observations":[{"d":"2024-01-02","FXUSDCAD":{"v":"1.3316"}}]} -> {date: rate}"""
    out = {}
    try:
        data = json.loads(text or "")
    except ValueError:
        return out
    for ob in (data or {}).get("observations") or []:
        if not isinstance(ob, dict):
            continue
        d = str(ob.get("d") or "")[:10]
        cell = ob.get("FXUSDCAD") or {}
        try:
            v = float((cell or {}).get("v"))
        except (TypeError, ValueError):
            continue
        if len(d) == 10 and v > 0:
            out[d] = v
    return out


def parse_fred_csv(text):
    """observation_date,SP500 rows; '.' marks a holiday and is skipped."""
    out = {}
    for line in str(text or "").splitlines():
        parts = line.split(",")
        if len(parts) < 2:
            continue
        d = parts[0].strip()
        raw = parts[1].strip()
        if len(d) != 10 or d[4] != "-" or d[7] != "-" or not raw or raw == ".":
            continue
        try:
            px = float(raw)
        except ValueError:
            continue
        if px > 0:
            out[d] = px
    return out


def parse_stooq_csv(text):
    """Date,Open,High,Low,Close,Volume -> {date: close}"""
    out = {}
    for line in str(text or "").splitlines()[1:]:
        parts = line.split(",")
        if len(parts) < 5:
            continue
        d = parts[0].strip()
        try:
            px = float(parts[4])
        except ValueError:
            continue
        if len(d) == 10 and px > 0:
            out[d] = px
    return out


def refresh_fx(ssl_context=None):
    """Fetch USD/CAD days after the newest stored one (with a small overlap)."""
    last = store.fx_last_date()
    if last:
        start = (date.fromisoformat(last) - timedelta(days=7)).isoformat()
    else:
        start = FX_START
    url = "%s?start_date=%s" % (BOC_URL, start)
    try:
        rates = parse_boc_json(_get_text(url, ssl_context))
    except Exception:
        return 0
    return store.upsert_fx_rates(rates)


def refresh_benchmark(ssl_context=None):
    """S&P 500 closes. FRED serves the trailing ten years in one file."""
    mapping = {}
    try:
        mapping = parse_fred_csv(_get_text(FRED_URL, ssl_context))
    except Exception:
        mapping = {}
    if not mapping:
        try:
            mapping = parse_stooq_csv(_get_text(STOOQ_URL, ssl_context))
        except Exception:
            mapping = {}
    if not mapping:
        return 0
    last = store.benchmark_last_date()
    if last:
        cutoff = (date.fromisoformat(last) - timedelta(days=7)).isoformat()
        mapping = {d: v for d, v in mapping.items() if d >= cutoff}
    return store.upsert_benchmark_prices(mapping)


BENCHMARKS = {"SP500": "S&P 500", "TSX": "S&P/TSX", "TSX60": "TSX 60"}
# Stored benchmark key -> TMX Money index symbol.
TMX_INDICES = {"TSX": "^TSX", "TSX60": "^TX60"}
TSX_SYMBOL = TMX_INDICES["TSX"]
TSX_START = "2016-01-01"


def refresh_tmx_index(key, ssl_context=None):
    """One TMX index's daily closes, appended from a week before the newest stored day."""
    last = store.benchmark_last_date(key)
    start = (date.fromisoformat(last) - timedelta(days=7)).isoformat() if last else TSX_START
    try:
        data = _post_json(TMX_URL, {"operationName": "getTimeSeriesData", "variables": {"symbol": TMX_INDICES[key], "freq": "day", "interval": 1, "start": start, "end": date.today().isoformat()}, "query": TMX_HISTORY_QUERY}, ssl_context, _TMX_HEADERS)
    except Exception:
        return 0
    mapping = {b["date"]: b["close"] for b in parse_tmx_history(data) if b.get("close")}
    if not mapping:
        return 0
    return store.upsert_benchmark_prices(mapping, symbol=key)


def refresh_tsx(ssl_context=None):
    """The S&P/TSX Composite and the S&P/TSX 60 from TMX Money."""
    return sum(refresh_tmx_index(key, ssl_context) for key in TMX_INDICES)


def _post_json(url, payload, ssl_context=None, headers=None):
    body = json.dumps(payload).encode("utf-8")
    hdrs = {"User-Agent": UA, "Content-Type": "application/json", "Accept": "*/*"}
    hdrs.update(headers or {})
    req = Request(url, data=body, headers=hdrs, method="POST")
    ctx = ssl_context or default_ssl_context()
    with urlopen(req, timeout=TIMEOUT_SEC, context=ctx) as resp:
        raw = resp.read()
    if raw[:2] == b"\x1f\x8b":
        try:
            raw = gzip.decompress(raw)
        except OSError:
            pass
    return json.loads(raw.decode("utf-8", "replace"))


_TMX_HEADERS = {"locale": "en", "Origin": "https://money.tmx.com", "Referer": "https://money.tmx.com/"}


def tmx_symbol(symbol):
    """Wealthsimple's Canadian tickers already match TMX Money's (no suffix)."""
    s = str(symbol or "").strip().upper()
    for suffix in (".TO", ".V", ".CN", ".NE"):
        if s.endswith(suffix):
            s = s[: -len(suffix)]
    return s


def tmx_record_symbol(symbol, exchange):
    """TMX Money symbol for a listing's declared distribution record: bare for
    TSX, TSX-V and CSE listings, ':AQL' for Cboe Canada (the former NEO) ones,
    which TMX carries only under that suffix."""
    s = tmx_symbol(symbol)
    if not s:
        return None
    if str(exchange or "").strip().upper() in CBOE_CANADA_EXCHANGES:
        return s + ":AQL"
    return s


def tmx_quote_symbol(symbol, exchange, currency):
    """TMX Money symbol for a listing, in the form its venue takes (tmx_form).
    None when TMX does not carry it (crypto, options, unknown venues)."""
    s = tmx_symbol(symbol)
    if not s or " " in s:
        return None
    form = tmx_form(exchange, currency)
    return s + form if form is not None else None


# TMX Money names a listing by its venue: bare for TSX and TSX-V, ':CNX' for the
# CSE, ':AQL' for Cboe Canada (the former NEO), ':US' for US exchanges. The venue
# in the security record picks the form (tmx_form). Every TMX query goes through
# tmx_lookup: when the record's form answers nothing, the other forms for the
# record's currency are asked for a quote, the one naming a matching venue is
# remembered for the symbol, and the query is repeated with it. No venue is a
# special case, and a record with a wrong or missing venue still resolves.
TMX_FORMS = {"CAD": ("", ":CNX", ":AQL"), "USD": (":US",)}
TMX_VENUE_OF_FORM = {"": ("TORONTO STOCK EXCHANGE", "TSX VENTURE"), ":CNX": ("CANADIAN SECURITIES EXCHANGE",), ":AQL": ("CBOE", "NEO"), ":US": ("NYSE", "NASDAQ", "NEW YORK")}
TMX_RESOLVE_RETRY_DAYS = 1


def tmx_form(exchange, currency):
    """TMX's symbol suffix for a listing venue, or None when TMX does not carry it."""
    ex = str(exchange or "").strip().upper()
    ccy = str(currency or "").strip().upper()
    if ex in US_EXCHANGES or (not ex and ccy == "USD"):
        return ":US"
    if ex in CBOE_CANADA_EXCHANGES:
        return ":AQL"
    if ex == "CSE":
        return ":CNX"
    if ex in ("TSX", "TSX-V", "TSXV"):
        return ""
    # a venue TMX does not name (an ATS such as Alpha, or none at all): start from
    # the currency's usual form and let tmx_lookup settle it
    if ccy == "CAD":
        return ""
    if ccy == "USD":
        return ":US"
    return None


def tmx_record_symbol(symbol, exchange):
    """TMX Money symbol for a Canadian listing's declared distribution record."""
    s = tmx_symbol(symbol)
    if not s:
        return None
    form = tmx_form(exchange, "CAD")
    return s + form if form is not None else None


def tmx_bare(key):
    return str(key or "").split(":", 1)[0]


def tmx_remembered(key):
    """The form TMX answered to for this symbol, when one has been remembered."""
    if not key or key.startswith("^"):
        return key
    v = store.get_meta("tmx_form:" + tmx_bare(key))
    return tmx_bare(key) + v[1:] if v.startswith("@") else key


def tmx_resolve(key, ssl_context=None, now=None):
    """Which of TMX's forms of a symbol answers, checked by the venue its quote
    names; remembered for good, and a miss remembered for a day. '' when none."""
    if not key or key.startswith("^"):
        return key
    bare = tmx_bare(key)
    suffix = key[len(bare):]
    forms = TMX_FORMS["USD"] if suffix == ":US" else TMX_FORMS["CAD"]
    forms = [suffix] + [f for f in forms if f != suffix] if suffix in forms else list(forms)   # the record's own form first
    meta_key = "tmx_form:" + bare
    v = store.get_meta(meta_key)
    if v.startswith("@"):
        return bare + v[1:]
    today = (now or datetime.now(timezone.utc)).date()
    if v.startswith("none@") and v[5:] > (today - timedelta(days=TMX_RESOLVE_RETRY_DAYS)).isoformat():
        return ""
    for form in forms:
        cand = bare + form
        try:
            q = ((_post_json(TMX_URL, {"operationName": "getQuoteBySymbol", "variables": {"symbol": cand, "locale": "en"}, "query": TMX_QUOTE_QUERY}, ssl_context, _TMX_HEADERS) or {}).get("data") or {}).get("getQuoteBySymbol") or {}
        except Exception:
            q = {}
        venue = str(q.get("exchangeName") or "").upper()
        if venue and any(v_ in venue for v_ in TMX_VENUE_OF_FORM[form]):
            store.set_meta(meta_key, "@" + form)
            return cand
    store.set_meta(meta_key, "none@" + today.isoformat())
    return ""


def tmx_lookup(key, fn, ssl_context=None):
    """(result, form) of fn(form): the remembered or given form first; when it
    answers nothing, the form TMX resolves for the symbol instead."""
    first = tmx_remembered(key)
    r = fn(first)
    if r or not key or key.startswith("^"):
        return r, first
    alt = tmx_resolve(key, ssl_context)
    if alt and alt != first:
        return fn(alt), alt
    return r, first


def is_canadian_listing(exchange, currency):
    ex = str(exchange or "").strip().upper()
    if ex:
        return ex in CANADIAN_EXCHANGES
    return str(currency or "").strip().upper() == "CAD"


def parse_tmx_quote(data):
    q = ((data or {}).get("data") or {}).get("getQuoteBySymbol") or {}
    if not isinstance(q, dict) or not q:
        return None
    ex = str(q.get("exDividendDate") or "")[:10]
    return {
        "price": q.get("price"),
        "priceChange": q.get("priceChange"),
        "percentChange": q.get("percentChange"),
        "prevClose": q.get("prevClose"),
        "currency": str(q.get("currency") or ""),
        "dividendAmount": q.get("dividendAmount"),
        "dividendFrequency": str(q.get("dividendFrequency") or ""),
        "exDividendDate": ex,
        "name": str(q.get("name") or ""),
        "exchange": str(q.get("exchangeName") or ""),
    }


def parse_tmx_dividends(data):
    block = ((data or {}).get("data") or {}).get("dividends") or {}
    rows = block.get("dividends") if isinstance(block, dict) else None
    out = []
    for r in rows or []:
        if not isinstance(r, dict):
            continue
        ex = str(r.get("exDate") or "")[:10]
        try:
            amt = float(r.get("amount"))
        except (TypeError, ValueError):
            continue
        if len(ex) == 10 and amt > 0:
            out.append({"exDate": ex, "payDate": str(r.get("payableDate") or "")[:10], "amount": amt, "currency": str(r.get("currency") or "")})
    return out


def fetch_tmx(symbol, ssl_context=None, exchange=None):
    """Quote + declared distribution history for one Canadian listing; the
    exchange picks the TMX symbol form (see tmx_record_symbol)."""
    sym = tmx_record_symbol(symbol, exchange)
    if not sym:
        return None, []
    quote, form = tmx_lookup(sym, lambda k: _tmx_quote(k, ssl_context), ssl_context)
    divs = []
    try:
        divs = parse_tmx_dividends(_post_json(TMX_URL, {"operationName": "getDividendsForSymbol", "variables": {"symbol": form, "page": 1, "batch": TMX_BATCH}, "query": TMX_DIVIDENDS_QUERY}, ssl_context, _TMX_HEADERS))
    except Exception:
        divs = []
    return quote, divs


def _tmx_quote(tmx_sym, ssl_context=None):
    try:
        return parse_tmx_quote(_post_json(TMX_URL, {"operationName": "getQuoteBySymbol", "variables": {"symbol": tmx_sym, "locale": "en"}, "query": TMX_QUOTE_QUERY}, ssl_context, _TMX_HEADERS))
    except Exception:
        return None


def fetch_tmx_quote(tmx_sym, ssl_context=None):
    return tmx_lookup(tmx_sym, lambda k: _tmx_quote(k, ssl_context), ssl_context)[0]


def _num(v, default=0.0):
    try:
        if v is None or v == "":
            return default
        return float(v)
    except (TypeError, ValueError):
        return default


_OCC_WORDY = re.compile(r"^([A-Z][A-Z0-9.]{0,9}) (\d{1,2})([A-Z]{3})(\d{2}) (\d+(?:\.\d+)?) (CALL|PUT|C|P)$")
_OCC_COMPACT = re.compile(r"^([A-Z][A-Z0-9.]{0,9}) (\d{6}[CP]\d{8})$")
_MONTHS = ("JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC")


def occ_code(symbol):
    """'QNC 20NOV26 3.00 CALL' -> 'QNC261120C00003000' (the OCC code Cboe keys its chains by)."""
    u = re.sub(r"\s+", " ", str(symbol or "").strip().upper())
    m = _OCC_COMPACT.match(u)
    if m:
        return m.group(1) + m.group(2)
    m = _OCC_WORDY.match(u)
    if not m or m.group(3) not in _MONTHS:
        return ""
    root, day, mon, yy, strike, right = m.groups()
    return "%s%s%02d%02d%s%08d" % (root, yy, _MONTHS.index(mon) + 1, int(day), right[0], int(round(float(strike) * 1000)))


def occ_root(code):
    m = re.match(r"^([A-Z][A-Z0-9.]{0,9})\d{6}[CP]\d{8}$", str(code or ""))
    return m.group(1) if m else ""


def quote_source(rec):
    """(source, key) for a held instrument, or None when no public source covers it.
    tmx: TMX Money symbol. cboe_ca: Cboe Canada symbol. coinbase: 'BTC-CAD' pair in
    the position's own currency. cboe_options: OCC code, US-listed underlyings only."""
    kind = str(rec.get("kind") or "Shares")
    sym = tmx_symbol(rec.get("symbol"))
    ccy = str(rec.get("currency") or "CAD").strip().upper()
    if not sym:
        return None
    if kind == "Crypto":
        return ("coinbase", "%s-%s" % (sym, ccy))
    if kind == "Options":
        code = occ_code(rec.get("symbol"))
        return ("cboe_options", code) if code and ccy == "USD" else None
    if kind != "Shares":
        return None
    if str(rec.get("exchange") or "").strip().upper() in CBOE_CANADA_EXCHANGES:
        return ("cboe_ca", sym)
    q = tmx_quote_symbol(rec.get("symbol"), rec.get("exchange"), rec.get("currency"))
    return ("tmx", q) if q else None


def parse_coinbase(text, pair=""):
    d = (json.loads(text or "{}") or {}).get("data") or {}
    px = _num(d.get("amount"), None)
    if not px or px <= 0:
        return None
    return {"price": px, "currency": str(d.get("currency") or pair.split("-")[-1])}


def parse_cboe_ca_quote(text):
    """Cboe Canada's own quote feed. Outside a session 'last' is 0: use the previous close."""
    d = (json.loads(text or "{}") or {}).get("data") or {}
    last = _num(d.get("last"), None)
    prev = _num(d.get("prev_close"), None)
    px = last if last and last > 0 else prev
    if not px or px <= 0:
        return None
    return {"price": px, "priceChange": _num(d.get("change"), None), "percentChange": _num(d.get("change_pct"), None), "prevClose": prev, "currency": "CAD", "name": str(d.get("company_name") or "")}


def parse_cboe_options(text):
    """OCC code -> row for one underlying's delayed chain."""
    d = (json.loads(text or "{}") or {}).get("data") or {}
    return {str(o.get("option") or ""): o for o in d.get("options") or [] if isinstance(o, dict)}


def option_mark(row):
    """Price of one contract per share: the bid/ask midpoint while both are quoted,
    else the last trade, else the previous close."""
    if not isinstance(row, dict):
        return None
    bid, ask = _num(row.get("bid"), 0.0), _num(row.get("ask"), 0.0)
    prev = _num(row.get("prev_day_close"), None)
    if bid > 0 and ask > 0:
        px = (bid + ask) / 2
    else:
        px = _num(row.get("last_trade_price"), None) or prev
    if not px or px <= 0:
        return None
    return {"price": px, "prevClose": prev, "priceChange": (px - prev) if prev else None, "percentChange": ((px / prev - 1) * 100) if prev else None, "currency": "USD"}


def fetch_coinbase_spot(pair, ssl_context=None):
    try:
        return parse_coinbase(_get_text(COINBASE_URL % pair, ssl_context), pair)
    except Exception:
        return None


def fetch_cboe_ca_quote(sym, ssl_context=None):
    try:
        return parse_cboe_ca_quote(_get_text(CBOE_CA_URL % sym, ssl_context))
    except Exception:
        return None


def fetch_cboe_option_chain(root, ssl_context=None):
    try:
        return parse_cboe_options(_get_text(CBOE_OPTIONS_URL % root, ssl_context))
    except Exception:
        return {}


def quote_symbols_needing_refresh(symbols, now=None, max_age_minutes=QUOTE_REFRESH_MINUTES):
    """[(stored symbol, source, key)] for held instruments whose quote is older than max_age."""
    now = now or datetime.now(timezone.utc)
    fetched = store.quote_fetched_at()
    out = []
    seen = set()
    for rec in symbols or []:
        sym = tmx_symbol(rec.get("symbol"))
        src = quote_source(rec)
        if not sym or not src or sym in seen:
            continue
        seen.add(sym)
        last = fetched.get(sym) or ""
        try:
            age = now - datetime.fromisoformat(last.replace("Z", "+00:00")) if last else None
        except ValueError:
            age = None
        if age is None or age > timedelta(minutes=max_age_minutes):
            out.append((sym, src[0], src[1]))
    return out


def refresh_quotes(symbols, ssl_context=None, now=None):
    """Live-ish prices for held positions, at most every QUOTE_REFRESH_MINUTES.
    Shares and ETFs from TMX Money or Cboe Canada, crypto from Coinbase in the
    position's currency, US-listed options from Cboe's delayed chains."""
    done = 0
    chains = {}
    for sym, source, key in quote_symbols_needing_refresh(symbols, now=now):
        rec = None
        if source == "tmx":
            rec = fetch_tmx_quote(key, ssl_context)
        elif source == "cboe_ca":
            rec = fetch_cboe_ca_quote(key, ssl_context)
        elif source == "coinbase":
            rec = fetch_coinbase_spot(key, ssl_context)
        elif source == "cboe_options":
            root = occ_root(key)
            if root not in chains:
                chains[root] = fetch_cboe_option_chain(root, ssl_context)
            rec = option_mark(chains[root].get(key))
        if rec and rec.get("price") is not None:
            rec = dict(rec, source=source)
            store.upsert_quote(sym, rec, source=source)
            if source == "cboe_options":
                record_option_bars(sym, rec["price"], now)
            done += 1
    return done


def stale_symbols(symbols, now=None):
    """Dividend-paying Canadian listings whose declared distribution record
    is older than RECORD_STALE_HOURS. The record has its own fetch stamp: the
    quote loop keeps quotes fresh every few minutes, and that must not make
    the fund's distribution history look fresh."""
    now = now or datetime.now(timezone.utc)
    fetched = store.distributions_fetched_at()
    out = []
    for rec in symbols or []:
        sym = tmx_symbol(rec.get("symbol"))
        if not sym or not is_canadian_listing(rec.get("exchange"), rec.get("currency")):
            continue
        last = fetched.get(sym) or ""
        try:
            age = now - datetime.fromisoformat(last.replace("Z", "+00:00")) if last else None
        except ValueError:
            age = None
        if age is None or age > timedelta(hours=RECORD_STALE_HOURS):
            out.append(sym)
    return out


def refresh_distributions(symbols=None, ssl_context=None, force=False, now=None):
    """Refresh quotes and declared distributions for the dividend payers."""
    recs = symbols or []
    now = now or datetime.now(timezone.utc)
    todo = [tmx_symbol(r.get("symbol")) for r in recs if is_canadian_listing(r.get("exchange"), r.get("currency"))] if force else stale_symbols(recs, now=now)
    exchanges = {tmx_symbol(r.get("symbol")): r.get("exchange") for r in recs}
    done = 0
    for sym in todo:
        exchange = exchanges.get(sym)
        quote, divs = fetch_tmx(sym, ssl_context, exchange=exchange)
        # A Cboe Canada listing's price comes from Cboe's own feed every minute;
        # TMX's delayed quote for it must not replace that, only its record is kept.
        if quote and str(exchange or "").strip().upper() not in CBOE_CANADA_EXCHANGES:
            store.upsert_quote(sym, quote)
        if divs:
            store.upsert_distributions(sym, divs)
        if quote or divs:
            store.mark_distributions_fetched(sym, now.strftime("%Y-%m-%dT%H:%M:%SZ"))
            done += 1
    return done


def benchmark_stale(today=None):
    """True when any index the page can show has no closes, or none within
    STALE_DAYS: a newly added index is fetched on the next check, not on the
    six-hour clock."""
    today = today or _today()
    limit = (today - timedelta(days=STALE_DAYS)).isoformat()
    for sym in store.BENCHMARK_SYMBOLS:
        last = store.benchmark_last_date(sym)
        if not last or last < limit:
            return True
    return False


def is_stale(today=None, symbols=None):
    today = today or _today()
    limit = (today - timedelta(days=STALE_DAYS)).isoformat()
    fx = store.fx_last_date()
    if (not fx or fx < limit) or benchmark_stale(today):
        return True
    return bool(stale_symbols(symbols or []))


def refresh_all(ssl_context=None, symbols=None):
    """Refresh FX, the benchmark and the declared distributions for the given
    payer symbols. Never raises; returns row counts written."""
    global _refreshing
    with _lock:
        if _refreshing:
            return {"fx": 0, "benchmark": 0, "skipped": True}
        _refreshing = True
    try:
        store.set_meta("market_attempt_at", datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
        return {
            "fx": refresh_fx(ssl_context),
            "benchmark": refresh_benchmark(ssl_context) + refresh_tsx(ssl_context),
            "distributions": refresh_distributions(symbols or [], ssl_context),
            "skipped": False,
        }
    finally:
        with _lock:
            _refreshing = False


BOC_PUBLISH_ET = (16, 30)


def fx_day_published_but_missing(now=None):
    """True once the Bank of Canada has published today's rate (16:30 Eastern on a
    weekday) and the stored table does not have it yet, so a trade made today is
    converted at its own day's rate the same afternoon."""
    now = now or datetime.now(timezone.utc)
    et = now.astimezone(ZoneInfo("America/Toronto"))
    if et.weekday() > 4 or (et.hour, et.minute) < BOC_PUBLISH_ET:
        return False
    return (store.fx_last_date() or "") < et.date().isoformat()


# --------------------------------------------------------------------------
# daily price history for the trade chart
# --------------------------------------------------------------------------


def parse_tmx_history(data):
    rows = ((data or {}).get("data") or {}).get("getTimeSeriesData") or []
    out = []
    for r in rows:
        if not isinstance(r, dict):
            continue
        d = str(r.get("dateTime") or "")[:10]
        if len(d) == 10:
            out.append({"date": d, "open": r.get("open"), "high": r.get("high"), "low": r.get("low"), "close": r.get("close"), "volume": r.get("volume")})
    out.sort(key=lambda b: b["date"])
    return out


def parse_cboe_ca_history(text):
    rows = (json.loads(text or "{}") or {}).get("data") or []
    out = []
    for r in rows:
        if not isinstance(r, dict):
            continue
        d = str(r.get("date") or "")[:10]
        if len(d) == 10:
            out.append({"date": d, "open": r.get("open"), "high": r.get("high"), "low": r.get("low"), "close": r.get("close"), "volume": r.get("volume")})
    out.sort(key=lambda b: b["date"])
    return out


def parse_coinbase_candles(text):
    """Coinbase Exchange candles, [time, low, high, open, close, volume] rows, oldest first."""
    rows = json.loads(text or "[]") or []
    out = {}
    for r in rows:
        if not isinstance(r, list) or len(r) < 6:
            continue
        try:
            t = int(r[0])
            lo, hi, op, cl, vol = (float(x) for x in r[1:6])
        except (TypeError, ValueError):
            continue
        if cl > 0:
            out[t] = {"time": t, "open": op, "high": hi, "low": lo, "close": cl, "volume": vol}
    return [out[k] for k in sorted(out)]


def coinbase_market(pair, ssl_context=None, now=None):
    """The Coinbase Exchange market for a 'SYM-CCY' pair, or '' when it does not
    trade there; remembered, a miss for a day. The USD market is a separate
    candidate in the history chain, not a fallback taken here."""
    pair = str(pair or "").strip().upper()
    if "-" not in pair:
        return ""
    meta_key = "coinbase_product:" + pair
    v = store.get_meta(meta_key)
    if v.startswith("@"):
        return v[1:]
    today = (now or datetime.now(timezone.utc)).date()
    if v.startswith("none@") and v[5:] > (today - timedelta(days=TMX_RESOLVE_RETRY_DAYS)).isoformat():
        return ""
    try:
        d = json.loads(_get_text(COINBASE_EXCHANGE_PRODUCT_URL % pair, ssl_context) or "{}") or {}
    except Exception:
        d = {}
    if str(d.get("id") or "").upper() == pair:
        store.set_meta(meta_key, "@" + pair)
        return pair
    store.set_meta(meta_key, "none@" + today.isoformat())
    return ""


def fetch_coinbase_candles(product, granularity, start_ts, end_ts, ssl_context=None):
    """Candles of `granularity` seconds over [start_ts, end_ts], fetched
    COINBASE_CANDLE_LIMIT at a time, a few spans in parallel."""
    span = COINBASE_CANDLE_LIMIT * granularity
    chunks = []
    cur = int(start_ts) // granularity * granularity
    while cur < end_ts:
        chunks.append((cur, min(cur + span, int(end_ts))))
        cur += span
    iso = lambda ts: datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    def one(c):
        try:
            return parse_coinbase_candles(_get_text(COINBASE_CANDLES_URL % (product, granularity, iso(c[0]), iso(c[1])), ssl_context))
        except Exception:
            return []
    out = {}
    with ThreadPoolExecutor(max_workers=4) as pool:
        for bars in pool.map(one, chunks):
            for b in bars:
                out[b["time"]] = b
    return [out[k] for k in sorted(out)]


def _rate_on_or_before(fx, day, days=7):
    d = datetime.strptime(day, "%Y-%m-%d").date()
    for i in range(days):
        r = fx.get((d - timedelta(days=i)).isoformat())
        if r and r > 0:
            return r
    return None


def in_position_currency(bars, bar_currency, currency):
    """Bars in the position's currency: unchanged when quoted in it; USD bars into
    CAD at the Bank of Canada rate of the bar's day. A bar whose day has no
    published rate within a week is dropped, never guessed. Anything else cannot
    be converted and yields nothing."""
    quote = str(bar_currency or "").upper()
    ccy = str(currency or "CAD").upper()
    if quote == ccy:
        return list(bars)
    if not (quote == "USD" and ccy == "CAD"):
        return []
    fx = store.fx_rates()
    out = []
    for b in bars:
        day = b["date"] if b.get("date") else datetime.fromtimestamp(b["time"], tz=timezone.utc).date().isoformat()
        rate = _rate_on_or_before(fx, day)
        if not rate:
            continue
        out.append(dict(b, open=b["open"] * rate, high=b["high"] * rate, low=b["low"] * rate, close=b["close"] * rate))
    return out


def chart_instrument(rec):
    """What the trade chart draws for an instrument: the instrument itself, or for an
    option contract its underlying stock, since no source keeps contract history."""
    if str(rec.get("kind") or "") == "Options":
        from model import underlying_symbol
        under = underlying_symbol(rec.get("symbol"))
        if under and under != "—":
            return {"symbol": under, "exchange": rec.get("exchange") or "", "currency": rec.get("currency") or "USD", "kind": "Shares"}
    return dict(rec)


def yahoo_root(symbol):
    return tmx_symbol(symbol).replace(".", "-")


def yahoo_forms(rec):
    """Yahoo symbols for a share listing, the venue's own suffix first, then the
    other venues of the listing's currency (a wrong or missing venue still finds it)."""
    root = yahoo_root(rec.get("symbol"))
    ccy = str(rec.get("currency") or "CAD").strip().upper()
    if not root or " " in root or ccy not in YAHOO_FORMS:
        return []
    first = YAHOO_SUFFIX.get(str(rec.get("exchange") or "").strip().upper())
    forms = list(YAHOO_FORMS[ccy])
    if first is not None and first in forms:
        forms = [first] + [f for f in forms if f != first]
    return [root + f for f in forms]


def history_candidates(rec):
    """Where an instrument's bars can come from, in order of preference: the
    first source with bars for the span wins, and the winner is remembered for
    the symbol (fetch_history). Shares and ETFs: TMX Money under the venue's
    form, then Yahoo Finance under each venue suffix of the currency. Crypto: the
    Coinbase Exchange market and the Yahoo pair in the position's currency, then
    the USD market and pair converted at the Bank of Canada rate. Nothing for an
    option contract itself (chart_instrument maps it to its underlying)."""
    kind = str(rec.get("kind") or "Shares")
    sym = tmx_symbol(rec.get("symbol"))
    ccy = str(rec.get("currency") or "CAD").strip().upper()
    if not sym:
        return []
    if kind == "Crypto":
        out = [("coinbase", "%s-%s" % (sym, ccy)), ("yahoo", "%s-%s" % (sym, ccy))]
        if ccy != "USD":
            out += [("coinbase", sym + "-USD"), ("yahoo", sym + "-USD")]
        return out
    if kind != "Shares":
        return []
    out = []
    tmx_key = tmx_quote_symbol(rec.get("symbol"), rec.get("exchange"), ccy)
    if tmx_key:
        out.append(("tmx", tmx_key))
    out += [("yahoo", f) for f in yahoo_forms(rec)]
    return out


def history_source(rec):
    """The preferred (source, key) for an instrument's bars, or None when no source covers it."""
    c = history_candidates(rec)
    return c[0] if c else None


def _bars_meta_key(rec):
    return "bars_source:" + tmx_symbol(rec.get("symbol"))


def ordered_candidates(rec):
    """history_candidates with the remembered winner first."""
    cands = history_candidates(rec)
    v = store.get_meta(_bars_meta_key(rec)) if cands else ""
    if "|" in v:
        win = tuple(v.split("|", 1))
        if win in cands:
            return [win] + [c for c in cands if c != win]
    return cands


def _remember_winner(rec, source, key):
    store.set_meta(_bars_meta_key(rec), "%s|%s" % (source, key))


def bar_currency(source, key, rec):
    """The currency a candidate's bars are quoted in: a crypto pair's quote
    currency, otherwise the listing's own."""
    if str(rec.get("kind") or "") == "Crypto" and "-" in key:
        return key.split("-", 1)[1]
    return str(rec.get("currency") or "CAD").upper()


def parse_yahoo_chart(text):
    """Bars from Yahoo's chart endpoint: [{time, day, minute, offset, open, high,
    low, close, volume}] in the exchange's local day and minute, oldest first;
    rows with no close are dropped."""
    d = json.loads(text or "{}") or {}
    results = ((d.get("chart") or {}).get("result") or [])
    if not results:
        return []
    r = results[0]
    ts = r.get("timestamp") or []
    q = ((r.get("indicators") or {}).get("quote") or [{}])[0] or {}
    meta = r.get("meta") or {}
    # Yahoo's gmtoffset is the offset today, not the bar's; the exchange's named
    # zone gives each bar its own (standard or daylight) local time
    tz = None
    try:
        from zoneinfo import ZoneInfo
        tz = ZoneInfo(str(meta.get("exchangeTimezoneName") or ""))
    except Exception:
        tz = None
    fixed = int(meta.get("gmtoffset") or 0)
    out = []
    for i, t in enumerate(ts):
        try:
            close = float((q.get("close") or [None])[i])
        except (TypeError, ValueError, IndexError):
            continue
        if not close or close <= 0:
            continue
        pick = lambda k: (lambda v: float(v) if v is not None else None)((q.get(k) or [None] * len(ts))[i])
        if tz is not None:
            local = datetime.fromtimestamp(int(t), tz=tz)
            off = int((local.utcoffset() or timedelta(0)).total_seconds())
        else:
            off = fixed
            local = datetime.fromtimestamp(int(t) + off, tz=timezone.utc)
        out.append({"time": int(t), "day": local.date().isoformat(), "minute": local.hour * 60 + local.minute, "offset": off,
                    "open": pick("open"), "high": pick("high"), "low": pick("low"), "close": close, "volume": pick("volume")})
    out.sort(key=lambda b: b["time"])
    return out


_yahoo_lock = threading.Lock()
_yahoo_next_at = 0.0
_yahoo_backoff_until = 0.0


def _yahoo_get(url, ssl_context=None, now=None):
    """One Yahoo request at a time, spaced YAHOO_MIN_INTERVAL_SEC apart; after a
    429 nothing is asked for YAHOO_BACKOFF_SEC. Raises on any failure."""
    global _yahoo_next_at, _yahoo_backoff_until
    import time as _time
    with _yahoo_lock:
        t = _time.monotonic()
        if t < _yahoo_backoff_until:
            raise RuntimeError("yahoo: backing off after 429")
        wait = _yahoo_next_at - t
        if wait > 0:
            _time.sleep(wait)
        _yahoo_next_at = _time.monotonic() + YAHOO_MIN_INTERVAL_SEC
        try:
            return _get_text(url, ssl_context, headers=YAHOO_HEADERS)
        except Exception as e:
            if getattr(e, "code", None) == 429:
                _yahoo_backoff_until = _time.monotonic() + YAHOO_BACKOFF_SEC
            raise


def fetch_yahoo(symbol, start_ts, end_ts, interval, ssl_context=None, now=None):
    """Yahoo bars for a symbol; a symbol Yahoo says it does not carry (404) is
    remembered for the day and not asked again."""
    today = (now or datetime.now(timezone.utc)).date().isoformat()
    miss_key = "yahoo_miss:" + str(symbol)
    if store.get_meta(miss_key) == today:
        return []
    try:
        return parse_yahoo_chart(_yahoo_get(YAHOO_CHART_URL % (symbol, int(start_ts), int(end_ts), interval), ssl_context))
    except Exception as e:
        if getattr(e, "code", None) == 404:
            store.set_meta(miss_key, today)
        return []


def _whole_bars(bars):
    """Only bars with an open, high and low: the chart draws candlesticks or nothing."""
    return [b for b in bars if b.get("open") is not None and b.get("high") is not None and b.get("low") is not None]


def fetch_daily_from(source, key, rec, start, end, ssl_context=None):
    """Daily bars of one candidate over [start, end], oldest first; [] when it has none."""
    start_ts = int(datetime.strptime(start, "%Y-%m-%d").replace(tzinfo=timezone.utc).timestamp())
    end_ts = int(datetime.strptime(end, "%Y-%m-%d").replace(tzinfo=timezone.utc).timestamp()) + 86400
    if source == "tmx":
        def daily(form):
            data = _post_json(TMX_URL, {"operationName": "getTimeSeriesData", "variables": {"symbol": form, "freq": "day", "interval": 1, "start": start, "end": end}, "query": TMX_HISTORY_QUERY}, ssl_context, _TMX_HEADERS)
            return parse_tmx_history(data)
        return _whole_bars(tmx_lookup(key, daily, ssl_context)[0])
    if source == "coinbase":
        if not coinbase_market(key, ssl_context):
            return []
        days = [dict(b, date=datetime.fromtimestamp(b["time"], tz=timezone.utc).date().isoformat()) for b in fetch_coinbase_candles(key, 86400, start_ts, end_ts, ssl_context)]
        return in_position_currency(days, bar_currency(source, key, rec), rec.get("currency"))
    if source == "yahoo":
        days = [{"date": b["day"], "open": b["open"], "high": b["high"], "low": b["low"], "close": b["close"], "volume": b["volume"]} for b in fetch_yahoo(key, start_ts, end_ts, "1d", ssl_context)]
        return in_position_currency(_whole_bars(days), bar_currency(source, key, rec), rec.get("currency"))
    return []


COVERAGE_SLACK_DAYS = 7   # a source covers a span when its first bar is within this of the span's start


def _pick_covering(rec, answers, span_start, first_of):
    """(bars, source) from the candidates' answers: the first whose bars reach
    back to the span's start (within COVERAGE_SLACK_DAYS), else the one reaching
    furthest back. A source with a late start never beats one that has the
    earlier days. The winner is remembered."""
    slack = timedelta(days=COVERAGE_SLACK_DAYS)
    best = None
    for source, key, bars in answers:
        if not bars:
            continue
        first = first_of(bars[0])
        if first <= span_start + slack:
            _remember_winner(rec, source, key)
            return bars, source
        if best is None or first < best[0]:
            best = (first, source, key, bars)
    if best:
        _remember_winner(rec, best[1], best[2])
        return best[3], best[1]
    return [], ""


def fetch_history(rec, start, end, ssl_context=None):
    """Daily bars for one instrument between two dates, oldest first, from the
    chain: the first candidate whose bars cover the span, else the one covering
    most of it (see _pick_covering); the winner is remembered."""
    span_start = datetime.strptime(start, "%Y-%m-%d")
    answers = []
    notes = []
    for source, key in ordered_candidates(rec):
        try:
            bars = fetch_daily_from(source, key, rec, start, end, ssl_context)
            notes.append("%s %s %d bars" % (source, key, len(bars)))
        except Exception as e:
            bars = []
            notes.append("%s %s %s" % (source, key, type(e).__name__))
        answers.append((source, key, bars))
        if bars and datetime.strptime(bars[0]["date"], "%Y-%m-%d") <= span_start + timedelta(days=COVERAGE_SLACK_DAYS):
            break   # covered: no need to ask the rest
    out = _pick_covering(rec, answers, span_start, lambda b: datetime.strptime(b["date"], "%Y-%m-%d"))
    if not out[0]:
        # an empty chart is a question someone will ask: say what every source answered
        sys.stderr.write("bagholder chart: %s (%s, %s) daily %s..%s: %s\n" % (rec.get("symbol"), rec.get("exchange") or "no venue", rec.get("currency"), start, end, "; ".join(notes) or "no source"))
    return out


def ensure_history(rec, start, end, ssl_context=None, now=None):
    """Stored bars for [start, end], fetching when the span was never fetched or
    the copy is older than HISTORY_STALE_HOURS and the span reaches the present."""
    now = now or datetime.now(timezone.utc)
    sym = tmx_symbol(rec.get("symbol"))
    start, end = str(start or "")[:10], str(end or "")[:10]
    if not sym or len(start) != 10 or len(end) != 10:
        return []
    last = store.history_fetch(sym)
    covered = bool(last) and last["start"] <= start
    fresh = False
    if last:
        try:
            fresh = now - datetime.fromisoformat(last["fetchedAt"].replace("Z", "+00:00")) < timedelta(hours=HISTORY_STALE_HOURS)
        except ValueError:
            fresh = False
    today = now.date().isoformat()
    needs_recent = end >= (now.date() - timedelta(days=3)).isoformat()
    if not covered or (needs_recent and not fresh):
        fetch_from = start if not covered else min(start, last["start"])
        bars, source = fetch_history(rec, fetch_from, today, ssl_context)
        if bars:
            store.upsert_price_history(sym, bars, source)
            # the stamp says what is covered: when the bars begin well after the
            # day asked for, only from their first day, so an earlier span asks again
            got_from = bars[0]["date"]
            covered_from = fetch_from if datetime.strptime(got_from, "%Y-%m-%d") <= datetime.strptime(fetch_from, "%Y-%m-%d") + timedelta(days=COVERAGE_SLACK_DAYS) else got_from
            store.mark_history_fetched(sym, covered_from, now.strftime("%Y-%m-%dT%H:%M:%SZ"))
    return store.price_history(sym, start, end)


def aggregate_daily(bars, tf):
    """Weekly (Monday start) or monthly bars from daily ones; open, high, low, close and
    volume are the period's first, max, min, last and sum."""
    out = []
    cur = None
    for b in bars:
        d = date.fromisoformat(b["date"])
        key = (d - timedelta(days=d.weekday())).isoformat() if tf == "1w" else d.replace(day=1).isoformat()
        if cur is None or cur["date"] != key:
            cur = {"date": key, "open": b.get("open"), "high": b.get("high"), "low": b.get("low"), "close": b["close"], "volume": b.get("volume")}
            out.append(cur)
            continue
        cur["close"] = b["close"]
        if b.get("high") is not None:
            cur["high"] = max(cur["high"], b["high"]) if cur["high"] is not None else b["high"]
        if b.get("low") is not None:
            cur["low"] = min(cur["low"], b["low"]) if cur["low"] is not None else b["low"]
        if b.get("volume") is not None:
            cur["volume"] = (cur["volume"] or 0) + b["volume"]
    return out


def parse_tmx_minutes(data):
    """One-minute bars from TMX's chart feed: [{time, open, high, low, close, volume, minute}],
    where `minute` is the exchange-local minute of day, oldest first."""
    rows = ((data or {}).get("data") or {}).get("intraday") or []
    out = []
    for r in rows:
        if not isinstance(r, dict) or not r.get("dateTime"):
            continue
        try:
            dt = datetime.fromisoformat(str(r["dateTime"]))
        except ValueError:
            continue
        close = _num(r.get("close"), None)
        if not close or close <= 0:
            continue
        out.append({"time": int(dt.timestamp()), "day": dt.date().isoformat(), "minute": dt.hour * 60 + dt.minute, "offset": int((dt.utcoffset() or timedelta(0)).total_seconds()),
                    "open": _num(r.get("open"), None), "high": _num(r.get("high"), None), "low": _num(r.get("low"), None), "close": close, "volume": _num(r.get("volume"), None)})
    out.sort(key=lambda b: b["time"])
    return out


def aggregate_session(minutes, bucket_minutes):
    """Bars of `bucket_minutes` aligned to the session open (9:30 exchange time),
    from one-minute bars: open, high, low, close and volume are the bucket's first,
    max, min, last and sum. The bar's time is the bucket's start."""
    out = {}
    for m in minutes:
        rel = m["minute"] - SESSION_OPEN_MINUTES
        if rel < 0:
            rel = 0
        idx = rel // bucket_minutes
        start_minute = SESSION_OPEN_MINUTES + idx * bucket_minutes
        key = (m["day"], idx)
        if key not in out:
            day = datetime.strptime(m["day"], "%Y-%m-%d")
            start = int((day + timedelta(minutes=start_minute)).replace(tzinfo=timezone.utc).timestamp()) - m["offset"]
            out[key] = {"time": start, "open": m["open"] if m["open"] is not None else m["close"], "high": m["high"] if m["high"] is not None else m["close"], "low": m["low"] if m["low"] is not None else m["close"], "close": m["close"], "volume": m["volume"] or 0}
            continue
        b = out[key]
        b["close"] = m["close"]
        if m["high"] is not None:
            b["high"] = max(b["high"], m["high"])
        if m["low"] is not None:
            b["low"] = min(b["low"], m["low"])
        b["volume"] = (b["volume"] or 0) + (m["volume"] or 0)
    return [out[k] for k in sorted(out, key=lambda k: out[k]["time"])]


def fetch_tmx_minutes(key, start, end, ssl_context=None):
    """One-minute bars over [start, end], fetched a month at a time, a few months in parallel."""
    chunks = []
    cur = datetime.strptime(str(start)[:10], "%Y-%m-%d").date()
    last = datetime.strptime(str(end)[:10], "%Y-%m-%d").date()
    while cur <= last:
        nxt = (cur.replace(day=1) + timedelta(days=32)).replace(day=1) - timedelta(days=1)
        chunks.append((cur.isoformat(), min(nxt, last).isoformat()))
        cur = min(nxt, last) + timedelta(days=1)
    def one(span):
        try:
            return parse_tmx_minutes(_post_json(TMX_URL, {"operationName": "getCompanyChart", "variables": {"symbol": key, "from": span[0], "to": span[1]}, "query": TMX_CHART_QUERY}, ssl_context, _TMX_HEADERS))
        except Exception:
            return []
    out = []
    with ThreadPoolExecutor(max_workers=4) as pool:
        for bars in pool.map(one, chunks):
            out.extend(bars)
    out.sort(key=lambda b: b["time"])
    return out


_pending_lock = threading.Lock()
_pending = set()


def intraday_ready(rec, tf, start, now=None):
    """True when the stored bars already cover [start, now] for this timeframe."""
    now = now or datetime.now(timezone.utc)
    sym = tmx_symbol(rec.get("symbol"))
    reach = intraday_reach(rec, now)
    if not sym or not reach or tf not in INTRADAY_SECONDS:
        return True
    if intraday_missed_recently(sym, tf, now):
        return True   # nothing to wait for: the last try produced nothing
    start_day = max(str(start)[:10], reach)
    start_ts = int(datetime.strptime(start_day, "%Y-%m-%d").replace(tzinfo=timezone.utc).timestamp())
    last = store.bar_fetch(sym, tf)
    return bool(last) and last["startTs"] <= start_ts


def ensure_intraday_in_background(rec, tf, start, end, ssl_context=None):
    """Start the fetch for a span that is not stored yet, once per instrument, and
    return at once. Callers poll intraday_ready."""
    sym = tmx_symbol(rec.get("symbol"))
    with _pending_lock:
        if sym in _pending:
            return
        _pending.add(sym)
    def run():
        try:
            ensure_intraday(rec, tf, start, end, ssl_context)
        finally:
            with _pending_lock:
                _pending.discard(sym)
    threading.Thread(target=run, name="bagholder-intraday-" + sym, daemon=True).start()


def session_bucket(now, bucket_minutes):
    """Start (unix seconds) of the session-aligned bucket containing `now`, or None
    outside the 9:30 to 16:00 Eastern session or on a weekend."""
    et = now.astimezone(ZoneInfo("America/New_York"))
    minute = et.hour * 60 + et.minute
    if et.weekday() > 4 or minute < SESSION_OPEN_MINUTES or minute >= 16 * 60:
        return None
    idx = (minute - SESSION_OPEN_MINUTES) // bucket_minutes
    start = et.replace(hour=0, minute=0, second=0, microsecond=0) + timedelta(minutes=SESSION_OPEN_MINUTES + idx * bucket_minutes)
    return int(start.timestamp())


def record_option_bars(symbol, price, now=None):
    """Fold a contract's observed mark into its own 1h and 4h bars, since no source
    keeps option history: the app records it while the contract is held."""
    now = now or datetime.now(timezone.utc)
    for tf, minutes in (("1h", 60), ("4h", 240)):
        ts = session_bucket(now, minutes)
        if ts is not None:
            store.record_bar_tick(symbol, tf, ts, price)


def aggregate_hourly(bars, seconds):
    """Hourly bars onto a coarser grid aligned to the clock: open, high, low,
    close and volume are the bucket's first, max, min, last and sum."""
    out = {}
    for b in bars:
        k = int(b["time"]) // seconds * seconds
        cur = out.get(k)
        hi, lo = b.get("high", b["close"]), b.get("low", b["close"])
        if cur is None:
            out[k] = {"time": k, "open": b.get("open", b["close"]), "high": hi, "low": lo, "close": b["close"], "volume": b.get("volume") or 0}
        else:
            cur["high"] = max(cur["high"], hi)
            cur["low"] = min(cur["low"], lo)
            cur["close"] = b["close"]
            cur["volume"] += b.get("volume") or 0
    return [out[k] for k in sorted(out)]


def source_intraday_reach(source, now=None):
    """Earliest date a source has intraday bars for."""
    now = now or datetime.now(timezone.utc)
    if source == "coinbase":
        return COINBASE_EXCHANGE_START
    days = SOURCE_INTRADAY_DAYS.get(source)
    return (now.date() - timedelta(days=days)).isoformat() if days else ""


def intraday_reach(rec, now=None):
    """Earliest date intraday bars exist for across the instrument's sources, or ''."""
    reaches = [source_intraday_reach(src, now) for src, _ in history_candidates(rec)]
    reaches = [r for r in reaches if r]
    return min(reaches) if reaches else ""


def available_timeframes(rec, start, now=None):
    """Timeframes the chart can show for a trade starting on `start`."""
    if not history_candidates(rec):
        return []
    out = []
    reach = intraday_reach(rec, now)
    if reach and str(start)[:10] >= reach:
        out += ["1h", "4h"]
    return out + ["1d", "1w", "1M"]


def fetch_intraday_from(source, key, rec, start_ts, end_ts, ssl_context=None):
    """{tf: bars} of one candidate over [start_ts, end_ts]. TMX: one-minute bars
    aggregated to session-aligned 1h and 4h. Yahoo: hourly bars, session-aligned
    for listings and on the clock for crypto, 4h from them. Coinbase Exchange:
    hourly candles, 4h on a four-hour grid."""
    crypto = str(rec.get("kind") or "") == "Crypto"
    if source == "tmx":
        start = datetime.fromtimestamp(start_ts, tz=timezone.utc).date().isoformat()
        end = datetime.fromtimestamp(end_ts, tz=timezone.utc).date().isoformat()
        minutes = tmx_lookup(key, lambda form: fetch_tmx_minutes(form, start, end, ssl_context), ssl_context)[0]
        return {"1h": aggregate_session(minutes, 60), "4h": aggregate_session(minutes, 240)} if minutes else {}
    if source == "coinbase":
        if not coinbase_market(key, ssl_context):
            return {}
        hourly = in_position_currency(fetch_coinbase_candles(key, 3600, start_ts, end_ts, ssl_context), bar_currency(source, key, rec), rec.get("currency"))
        return {"1h": hourly, "4h": aggregate_hourly(hourly, 14400)} if hourly else {}
    if source == "yahoo":
        hourly = in_position_currency(_whole_bars(fetch_yahoo(key, start_ts, end_ts, "60m", ssl_context)), bar_currency(source, key, rec), rec.get("currency"))
        if not hourly:
            return {}
        if crypto:
            return {"1h": [{k: b[k] for k in ("time", "open", "high", "low", "close", "volume")} for b in hourly], "4h": aggregate_hourly(hourly, 14400)}
        return {"1h": aggregate_session(hourly, 60), "4h": aggregate_session(hourly, 240)}
    return {}


ON_DEMAND_ONLY_SOURCES = ("yahoo",)   # rate-limited: asked for a chart someone opens, never by the background sweep
INTRADAY_RETRY_MINUTES = 10


def _miss_key(symbol, tf):
    return "bars_miss:%s|%s" % (tmx_symbol(symbol), tf)


def record_intraday_miss(symbol, tf, now=None):
    """A fetch that produced no bars for this timeframe: remembered so the page
    stops asking and the sources are left alone until INTRADAY_RETRY_MINUTES pass."""
    now = now or datetime.now(timezone.utc)
    store.set_meta(_miss_key(symbol, tf), now.strftime("%Y-%m-%dT%H:%M:%SZ"))


def intraday_missed_recently(symbol, tf, now=None):
    now = now or datetime.now(timezone.utc)
    v = store.get_meta(_miss_key(symbol, tf))
    if not v:
        return False
    try:
        return now - datetime.fromisoformat(v.replace("Z", "+00:00")) < timedelta(minutes=INTRADAY_RETRY_MINUTES)
    except ValueError:
        return False


def offered_timeframes(rec, start, now=None):
    """available_timeframes less any intraday timeframe a recent fetch could not
    supply and nothing is stored for: the chart falls back to daily bars instead
    of waiting on a source that has just said no."""
    out = available_timeframes(rec, start, now)
    sym = tmx_symbol(rec.get("symbol"))
    return [tf for tf in out if tf not in INTRADAY_SECONDS or not intraday_missed_recently(sym, tf, now) or store.price_bars(sym, tf, 0, 2 ** 40)]


def fetch_intraday(rec, start_ts, end_ts, ssl_context=None, on_demand=True):
    """{tf: bars} over [start_ts, end_ts] from the first candidate whose reach
    covers the span and that has bars for it; the remembered winner is tried
    first. A source whose reach stops short of the span is skipped rather than
    asked for a partial answer. The background sweep (on_demand=False) leaves
    the rate-limited sources alone."""
    start_day = datetime.fromtimestamp(start_ts, tz=timezone.utc).date().isoformat()
    span_start = datetime.fromtimestamp(start_ts, tz=timezone.utc).replace(tzinfo=None)
    answers = []
    notes = []
    for source, key in ordered_candidates(rec):
        reach = source_intraday_reach(source)
        if not reach or start_day < reach or (not on_demand and source in ON_DEMAND_ONLY_SOURCES):
            notes.append("%s %s skipped" % (source, key))
            continue
        try:
            by_tf = fetch_intraday_from(source, key, rec, start_ts, end_ts, ssl_context)
            notes.append("%s %s %d bars" % (source, key, len(by_tf.get("1h") or [])))
        except Exception as e:
            by_tf = {}
            notes.append("%s %s %s" % (source, key, type(e).__name__))
        answers.append((source, key, by_tf.get("1h") or [], by_tf))
        if by_tf.get("1h") and datetime.fromtimestamp(by_tf["1h"][0]["time"], tz=timezone.utc).replace(tzinfo=None) <= span_start + timedelta(days=COVERAGE_SLACK_DAYS):
            break
    bars, source = _pick_covering(rec, [(a[0], a[1], a[2]) for a in answers], span_start, lambda b: datetime.fromtimestamp(b["time"], tz=timezone.utc).replace(tzinfo=None))
    if not bars:
        if on_demand:
            sys.stderr.write("bagholder chart: %s (%s, %s) hourly from %s: %s\n" % (rec.get("symbol"), rec.get("exchange") or "no venue", rec.get("currency"), start_day, "; ".join(notes) or "no source"))
        return {}, ""
    return next(a[3] for a in answers if a[0] == source and a[2] is bars), source


def ensure_intraday(rec, tf, start, end, ssl_context=None, now=None, max_age_hours=1, on_demand=True):
    """Stored bars of an intraday timeframe for [start, end]. Fetched from `start`
    when that span was never fetched; topped up from the last stored bar when the
    span reaches the present and the copy is older than `max_age_hours`. Bars once
    stored are kept for good, so a trade keeps its intraday chart as it ages past
    the source's reach."""
    now = now or datetime.now(timezone.utc)
    sym = tmx_symbol(rec.get("symbol"))
    reach = intraday_reach(rec, now)
    if not sym or not reach or tf not in INTRADAY_SECONDS:
        return []
    start_day = max(str(start)[:10], reach)
    start_ts = int(datetime.strptime(start_day, "%Y-%m-%d").replace(tzinfo=timezone.utc).timestamp())
    end_ts = min(int(datetime.strptime(str(end)[:10], "%Y-%m-%d").replace(tzinfo=timezone.utc).timestamp()) + 86400, int(now.timestamp()))
    last = store.bar_fetch(sym, tf)
    covered = bool(last) and last["startTs"] <= start_ts
    fresh = False
    if last:
        try:
            fresh = now - datetime.fromisoformat(last["fetchedAt"].replace("Z", "+00:00")) < timedelta(hours=max_age_hours)
        except ValueError:
            fresh = False
    needs_recent = end_ts >= int(now.timestamp()) - 3 * 86400
    fetch_from = None
    if not covered:
        fetch_from = start_ts
    elif needs_recent and not fresh:
        stored = store.price_bars(sym, tf, 0, 2 ** 40)
        fetch_from = max(start_ts, (stored[-1]["time"] if stored else start_ts) - 2 * 86400)
    if fetch_from is not None:
        by_tf, source = fetch_intraday(rec, fetch_from, int(now.timestamp()), ssl_context, on_demand=on_demand)
        for k in INTRADAY_SECONDS:   # one fetch fills every intraday timeframe, so a miss covers them all
            if not by_tf.get(k):
                record_intraday_miss(sym, k, now)
        for k, bars in by_tf.items():
            if bars:
                store.upsert_price_bars(sym, k, bars, source)
                covered_from = fetch_from if bars[0]["time"] <= fetch_from + COVERAGE_SLACK_DAYS * 86400 else bars[0]["time"]
                store.mark_bars_fetched(sym, k, min(covered_from, last["startTs"]) if last else covered_from, now.strftime("%Y-%m-%dT%H:%M:%SZ"))
    return store.price_bars(sym, tf, start_ts, end_ts)


ARCHIVE_BATCH = 12
ARCHIVE_TOPUP_HOURS = 20


def archive_intraday(recs, ssl_context=None, now=None, limit=ARCHIVE_BATCH):
    """Keep the intraday bars of recently traded or held instruments for good.
    Each call handles at most `limit` instruments: those never fetched first, then
    those whose copy is older than ARCHIVE_TOPUP_HOURS. Returns the symbols worked."""
    now = now or datetime.now(timezone.utc)
    todo = []
    for rec in recs or []:
        sym = tmx_symbol(rec.get("symbol"))
        if not sym or not intraday_reach(rec, now):
            continue
        last = store.bar_fetch(sym, "1h")
        age = None
        if last:
            try:
                age = now - datetime.fromisoformat(last["fetchedAt"].replace("Z", "+00:00"))
            except ValueError:
                age = None
        if last is None:
            todo.append((0, sym, rec))
        elif age is None or age > timedelta(hours=ARCHIVE_TOPUP_HOURS):
            todo.append((1, sym, rec))
    todo.sort(key=lambda x: (x[0], x[1]))
    done = []
    for _, sym, rec in todo[:limit]:
        ensure_intraday(rec, "1h", rec.get("start") or now.date().isoformat(), now.date().isoformat(), ssl_context, now, max_age_hours=ARCHIVE_TOPUP_HOURS, on_demand=False)
        done.append(sym)
    return done


SHORT_DAILY_SOURCES = ()


def archive_daily(recs, ssl_context=None, now=None, limit=ARCHIVE_BATCH):
    """Keep daily bars for instruments whose source forgets them. None of the
    current sources does (TMX and Coinbase keep full history), so this is idle
    until a source that forgets is added to SHORT_DAILY_SOURCES."""
    now = now or datetime.now(timezone.utc)
    todo = []
    for rec in recs or []:
        src = history_source(rec)
        sym = tmx_symbol(rec.get("symbol"))
        if not src or src[0] not in SHORT_DAILY_SOURCES or not sym:
            continue
        last = store.history_fetch(sym)
        age = None
        if last:
            try:
                age = now - datetime.fromisoformat(last["fetchedAt"].replace("Z", "+00:00"))
            except ValueError:
                age = None
        if last is None:
            todo.append((0, sym, rec))
        elif age is None or age > timedelta(hours=ARCHIVE_TOPUP_HOURS):
            todo.append((1, sym, rec))
    todo.sort(key=lambda x: (x[0], x[1]))
    done = []
    for _, sym, rec in todo[:limit]:
        ensure_history(rec, rec.get("start") or now.date().isoformat(), now.date().isoformat(), ssl_context, now)
        done.append(sym)
    return done


def ensure_bars(rec, tf, start, end, ssl_context=None, now=None):
    """Bars for one timeframe over a span: daily from the daily store, weekly and
    monthly aggregated from it, 1h and 4h from the intraday store."""
    if tf in INTRADAY_SECONDS:
        return ensure_intraday(rec, tf, start, end, ssl_context, now)
    daily = ensure_history(rec, start, end, ssl_context, now)
    return daily if tf == "1d" else aggregate_daily(daily, tf)


def refresh_periodic(ssl_context=None, symbols=None, now=None):
    """What the background loop runs every few minutes: USD/CAD and the
    S&P 500 at most every MARKET_ATTEMPT_HOURS, and the declared distribution
    record of every payer whose copy is older than RECORD_STALE_HOURS.
    Never raises; returns row counts written."""
    global _refreshing
    with _lock:
        if _refreshing:
            return {"fx": 0, "benchmark": 0, "distributions": 0, "skipped": True}
        _refreshing = True
    try:
        now = now or datetime.now(timezone.utc)
        out = {"fx": 0, "benchmark": 0, "distributions": 0, "skipped": False}
        last = store.get_meta("market_attempt_at")
        try:
            age = now - datetime.fromisoformat(last.replace("Z", "+00:00")) if last else None
        except ValueError:
            age = None
        if age is None or age > timedelta(hours=MARKET_ATTEMPT_HOURS) or fx_day_published_but_missing(now) or benchmark_stale(now.date()):
            store.set_meta("market_attempt_at", now.strftime("%Y-%m-%dT%H:%M:%SZ"))
            out["fx"] = refresh_fx(ssl_context)
            out["benchmark"] = refresh_benchmark(ssl_context) + refresh_tsx(ssl_context)
        out["distributions"] = refresh_distributions(symbols or [], ssl_context, now=now)
        return out
    finally:
        with _lock:
            _refreshing = False


def refresh_in_background(ssl_context=None, symbols=None):
    t = threading.Thread(
        target=refresh_all, args=(ssl_context, symbols), name="bagholder-market", daemon=True
    )
    t.start()
    return t
