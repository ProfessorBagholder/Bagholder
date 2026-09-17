"""Short selling for one listing, from the regulator that publishes it: FINRA for a US
listing, CIRO for a Canadian one.

Two measures are kept apart, because they answer different questions and are never added
together or folded into one number:

  the short position   the shares still sold short, as the dealers reported them, which
                       both regulators publish twice a month, on the 15th and the last
                       day of each month;
  the short volume     the part of a period's trading that was sold short, which the US
                       publishes for every trading day and Canada for each half-month.

Neither is live, and nothing published anywhere is: a position is a twice-monthly report
on both markets, and the US daily volume file, out about an hour after the close, is the
closest reading that exists without a securities-lending licence. Nothing here is
estimated or filled in — every figure is the regulator's own. Days to cover is the one
derived number, and it is derived the same way on both markets, from the app's own volume
history, rather than taking each regulator's own arithmetic where it happens to publish
some.
"""
from __future__ import annotations

import csv
import io
import json
import sys
import threading
import time
from datetime import date, datetime, timedelta, timezone

import edgar
import exposure
import instruments
import market
import model
import store
import xls

US_POSITION_URL = "https://api.finra.org/data/group/otcMarket/name/consolidatedShortInterest"
US_VOLUME_URL = "https://cdn.finra.org/equity/regsho/daily/CNMSshvol%s.txt"
CA_POSITION_URL = "https://www.ciro.ca/sites/default/files/epubs/CSPR/%s_CSPR_Report.xls"
CA_VOLUME_URL = "https://www.ciro.ca/sites/default/files/epubs/SSALE/%s-%s_ShortSaleTradingSummaryReport.csv"
CA_CBOE_URL = "https://www-api.cboe.com/ca/equities/listing-directory-data/"
CBOE_FUNDS = ("etf", "cef")      # what that venue calls the listings whose units in issue are their float
# what the Canadian files call each venue, against what the app calls it
CA_VENUES = {"TSX": ("TSX",), "TSXV": ("TSX-V", "TSXV"), "CSE": ("CSE",), "AQL": ("CBOE CANADA", "NEO")}
# and how the app writes each of them, for a listing the app knew no venue for
CA_VENUE_NAMES = {"TSX": "TSX", "TSXV": "TSX-V", "CSE": "CSE", "AQL": "Cboe Canada"}
FILE_HOURS = 6           # how often a whole-market file is looked for again
TRIES = 6                # how many report dates back to try before giving up
SERIES = 8               # reports behind the run shown with the position
FLOAT_HOURS = 12         # how often a float that answered is looked up again
FLOAT_MISS_MIN = 20      # a lookup that answered with nothing is tried again far sooner: a float
                         # that did not arrive is usually the source being slow, not the figure
                         # being absent, and holding the miss as long as a hit hides it for hours
YAHOO_QUOTE_URL = "https://finance.yahoo.com/quote/%s/"
YAHOO_CRUMB_URL = "https://query1.finance.yahoo.com/v1/test/getcrumb"
YAHOO_STATS_URL = "https://query1.finance.yahoo.com/v10/finance/quoteSummary/%s?modules=defaultKeyStatistics&crumb=%s"
TMX_UNITS_QUERY = ("query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) "
                   "{ symbol shareOutStanding } }")
HEADERS = {"User-Agent": market.UA, "Accept": "*/*"}

_files = {}
_shares = {}
_lock = threading.Lock()


def _s(v):
    return "" if v is None else str(v)


def _num(v):
    try:
        return float(_s(v).replace(",", "").strip())
    except ValueError:
        return None


# --- which regulator, if any, publishes for a listing -----------------------------

def market_of(symbol, exchange, currency):
    """'us', 'ca', or '' for an instrument no one reports short selling on: a coin, an
    index, a futures or currency contract, an option. The venue is read through the same
    routing the quotes use, so a listing resolves here exactly as it does everywhere."""
    sym = _s(symbol).strip().upper()
    ex = _s(exchange).strip().upper()
    if not sym or model.is_option_symbol(sym) or ex == "CRYPTO" or instruments.find(sym, ex):
        return ""
    form = market.tmx_form(ex, currency)
    if form is None:
        return ""
    return "us" if form == ":US" else "ca"


# --- when each report is for -------------------------------------------------------

def position_dates(today, back=TRIES):
    """The reporting dates of the twice-monthly position reports, newest first. Each is
    published a few days after the date it is for, so the newest that answers is the
    newest there is; asking is how we find out, since neither regulator says."""
    out, year, month = [], today.year, today.month
    while len(out) < back:
        last = date(year + (month == 12), (month % 12) + 1, 1) - timedelta(days=1)
        for d in (last, date(year, month, 15)):
            if d <= today and len(out) < back:
                out.append(d)
        year, month = (year - 1, 12) if month == 1 else (year, month - 1)
    return out


def volume_periods(today, back=TRIES):
    """The half-month periods the Canadian volume report covers, newest first."""
    out, year, month = [], today.year, today.month
    while len(out) < back:
        last = date(year + (month == 12), (month % 12) + 1, 1) - timedelta(days=1)
        for start, end in ((date(year, month, 16), last), (date(year, month, 1), date(year, month, 15))):
            if end <= today and len(out) < back:
                out.append((start, end))
        year, month = (year - 1, 12) if month == 1 else (year, month - 1)
    return out


def trading_days(today, back=TRIES):
    """The days the US volume file could be for, newest first: weekdays. A holiday has
    no file and is skipped by the asking, as the day before the file is out is."""
    out, d = [], today
    while len(out) < back:
        if d.weekday() < 5:
            out.append(d)
        d -= timedelta(days=1)
    return out


# --- the whole-market files, read once and kept ------------------------------------

def _table(name, build, ssl_context, now):
    """A file every listing is looked up in, fetched at most once every FILE_HOURS. A
    fetch that fails keeps what was already read rather than emptying it."""
    with _lock:
        held = _files.get(name)
        if held and time.time() - held["at"] < FILE_HOURS * 3600:
            return held
    try:
        built = build(ssl_context, now)
    except Exception as e:                                      # pragma: no cover - defensive
        sys.stderr.write("bagholder shorts: %s failed: %s\n" % (name, e))
        built = None
    with _lock:
        if built is None:
            held = _files.get(name) or {"key": "", "rows": {}}
            held["at"] = time.time()
        else:
            held = {"key": built[0], "rows": built[1], "at": time.time()}
        _files[name] = held
        return held


def parse_us_volume(text):
    """FINRA's daily file: one pipe-separated line per symbol, a trailer at the end."""
    rows = {}
    for line in _s(text).splitlines()[1:]:
        parts = line.strip().split("|")
        if len(parts) < 5:
            continue
        sym, short, total = parts[1].strip().upper(), _num(parts[2]), _num(parts[4])
        if sym and short is not None and total:
            rows[sym] = {"shortVolume": short, "totalVolume": total}
    return rows


def _us_volume_file(ssl_context, now):
    for d in trading_days(now.date()):
        url = US_VOLUME_URL % d.strftime("%Y%m%d")
        try:
            rows = parse_us_volume(market._get_text(url, ssl_context, HEADERS))
        except Exception:
            continue
        if rows:
            return d.strftime("%Y-%m-%d"), rows
    return None


def parse_ca_positions(grid):
    """CIRO's position report: issue name, symbol, venue, shares short, net change."""
    rows = {}
    for r in grid or []:
        if len(r) < 5:
            continue
        sym, shares = _s(r[1]).strip().upper(), _num(r[3])
        if not sym or shares is None:
            continue
        rows[sym] = {"venue": _s(r[2]).strip().upper(), "shares": shares, "change": _num(r[4]), "name": _s(r[0]).strip()}
    return rows


def _ca_position_file(ssl_context, now):
    for d in position_dates(now.date()):
        url = CA_POSITION_URL % d.strftime("%Y%m%d")
        try:
            raw = market._fetch(url, ssl_context or market.default_ssl_context(), HEADERS, market.TIMEOUT_SEC)
            rows = parse_ca_positions(xls.table(raw))
        except Exception:
            market.note_source("ciro", False, "no report for %s" % d)
            continue
        if rows:
            market.note_source("ciro", True)
            return d.strftime("%Y-%m-%d"), rows
    return None


def ca_traded(symbol, exchange, currency, span, ssl_context=None):
    """A Canadian listing's own volume over the report's period, from TMX's daily series under
    the venue's own form. CIRO's volume report lists only the securities that were sold short,
    so a listing with none is absent from it and has no total there to measure days to cover
    against — but it traded, and the exchange says how much."""
    start, end = (_s(span).split("/", 1) + [""])[:2]
    if not end:
        return None
    try:
        code = market.tmx_quote_symbol(symbol, exchange, currency)
        if not code:
            return None
        def ask(form):
            data = market._post_json(market.TMX_URL, {"operationName": "getTimeSeriesData",
                                                      "variables": {"symbol": form, "freq": "day", "interval": 1, "start": start, "end": end},
                                                      "query": market.TMX_HISTORY_QUERY}, ssl_context, market._TMX_HEADERS)
            return market.parse_tmx_history(data)
        bars = market.tmx_lookup(code, ask, ssl_context)[0] or []
        traded = [b.get("volume") for b in bars if b.get("volume")]
        return sum(traded) if traded else None
    except Exception as e:
        sys.stderr.write("bagholder shorts: %s traded volume failed: %s\n" % (symbol, e))
        return None


def parse_ca_volume(text):
    """CIRO's short sale summary: the short part of a period's trading, per listing."""
    rows = {}
    for r in csv.DictReader(io.StringIO(_s(text).lstrip("﻿"))):
        sym = _s(r.get("Security")).strip().upper()
        short, pct = _num(r.get("Short Traded Volume")), _num(r.get("% Total Traded Volume"))
        if not sym or short is None:
            continue
        rows[sym] = {"venue": _s(r.get("Listing Market")).strip().upper(), "shortVolume": short, "volumePct": pct,
                     "totalVolume": (short / pct * 100) if pct else None}
    return rows


def _ca_volume_file(ssl_context, now):
    for start, end in volume_periods(now.date()):
        url = CA_VOLUME_URL % (start.strftime("%Y%m%d"), end.strftime("%Y%m%d"))
        try:
            rows = parse_ca_volume(market._get_text(url, ssl_context, HEADERS))
        except Exception:
            continue
        if rows:
            return "%s/%s" % (start.isoformat(), end.isoformat()), rows
    return None


def _ca_positions_on(day, ssl_context=None):
    """One dated Canadian report, kept for the session so a run of them is read once."""
    name = "ca_position:" + day
    with _lock:
        held = _files.get(name)
    if held:
        return held["rows"]
    try:
        raw = market._fetch(CA_POSITION_URL % day.replace("-", ""), ssl_context or market.default_ssl_context(), HEADERS, market.TIMEOUT_SEC)
        rows = parse_ca_positions(xls.table(raw))
    except Exception:
        rows = {}
    with _lock:
        _files[name] = {"key": day, "rows": rows, "at": time.time()}
    return rows


def ca_series(symbol, exchange="", asof="", ssl_context=None, now=None, back=SERIES):
    """The listing's position across the last reports, oldest first. Canada publishes one
    file per reporting date rather than a run of them, so each is read on its own and kept."""
    sym, out = _s(symbol).strip().upper(), []
    for d in position_dates((now or datetime.now(timezone.utc)).date(), back=back):
        day = d.isoformat()
        if asof and day > asof:
            continue
        row = _ca_positions_on(day, ssl_context).get(sym)
        if row and _venue_fits(row.get("venue"), exchange):
            out.append({"date": day, "shares": row["shares"]})
    return sorted(out, key=lambda x: x["date"])


_yahoo = {"session": None, "crumb": ""}


def _yahoo_session():
    """A session that can read Yahoo's statistics. Its own TLS handshake is the gate: the
    standard library is refused there whatever headers it sends, and `curl_cffi` — already
    the app's one optional dependency, for SEDAR+ — presents a browser's. Without it the
    float is simply unknown, as it is for a listing Yahoo does not carry."""
    if _yahoo["session"] is not None:
        return _yahoo["session"], _yahoo["crumb"]
    try:
        from curl_cffi import requests as cffi
    except Exception:
        return None, ""
    try:
        session = cffi.Session(impersonate="chrome")
        session.get(YAHOO_QUOTE_URL % "AAPL", timeout=market.TIMEOUT_SEC)
        crumb = _s(session.get(YAHOO_CRUMB_URL, timeout=market.TIMEOUT_SEC).text).strip()
        if not crumb or len(crumb) > 32:
            return None, ""
        _yahoo["session"], _yahoo["crumb"] = session, crumb
        return session, crumb
    except Exception as e:
        sys.stderr.write("bagholder shorts: yahoo would not open: %s\n" % e)
        return None, ""


def _cboe_units(symbol, ssl_context=None, now=None):
    """The units a fund listed on Cboe Canada has in issue, from that venue's own directory.
    The directory publishes each listing's market capitalisation beside its last price, and a
    capitalisation is the count times that price: dividing gives back the count the exchange
    put in, whole for every listing it carries, which is the figure itself and not one rounded
    into shape. It is asked only for the venue's funds: for a company the shares in issue are
    not the float, and Yahoo publishes that. TMX carries Cboe listings but answers 0 for their
    counts, and Yahoo publishes no count for a Canadian fund, so for these listings this is the
    only place the figure exists."""
    def build(ctx, when):
        rows = {}
        for r in (json.loads(market._get_text(CA_CBOE_URL, ctx, headers=HEADERS)) or {}).get("data") or []:
            if _s(r.get("security")).strip().lower() not in CBOE_FUNDS:
                continue
            cap, last = _num(r.get("marketcap")), _num(r.get("last"))
            if not cap or not last:
                continue
            count = cap / last
            if abs(count - round(count)) < 1e-6:     # anything else is not the exchange's own count
                rows[_s(r.get("symbol")).strip().upper()] = float(round(count))
        return ("cboe", rows)
    return _table("cboe_listings", build, ssl_context, now)["rows"].get(_s(symbol).strip().upper())


def _fund_units(symbol, exchange, currency, ssl_context=None):
    """The units an exchange-traded fund has in issue, from the market's own source. A fund
    creates and redeems units on demand and holds none back, so the units in issue are the
    units there are to trade: for a fund this is the float, not a stand-in for it. TMX answers
    for the venues it carries counts for, Cboe Canada's own directory for its listings."""
    sym = _s(symbol).strip().upper()
    if market_of(sym, exchange, currency) == "us":
        return None                              # the US count comes from Yahoo with the float below
    count = None
    try:
        code = market.tmx_quote_symbol(sym, exchange, currency)
        if code:
            def ask(form):
                answered = market._post_json(market.TMX_URL, {"operationName": "getQuoteBySymbol", "variables": {"symbol": form, "locale": "en"},
                                                              "query": TMX_UNITS_QUERY}, ssl_context, market._TMX_HEADERS)
                return ((answered or {}).get("data") or {}).get("getQuoteBySymbol") or {}
            count = _num((market.tmx_lookup(code, ask, ssl_context)[0] or {}).get("shareOutStanding")) or None
    except Exception as e:
        sys.stderr.write("bagholder shorts: %s units failed: %s\n" % (sym, e))
    if count:
        return count
    # the venue is asked for its own listings only: a symbol is one company's on one venue and
    # another's on the next, and a count taken from the wrong venue would be the wrong fund's
    if _s(exchange).strip().upper() not in CA_VENUES["AQL"]:
        return None
    try:
        return _cboe_units(sym, ssl_context)
    except Exception as e:
        sys.stderr.write("bagholder shorts: %s units from cboe failed: %s\n" % (sym, e))
        return None


def _yahoo_paced(call):
    """Yahoo at the pace the rest of the app already keeps with it: one request at a time,
    spaced, and none at all while a backoff after a 429 stands. These lookups are a burst by
    nature — one per listing whenever the sweep runs — and unpaced they were the burst Yahoo
    turned away, which read as a listing having no float when it has one."""
    with market._yahoo_lock:
        if time.monotonic() < market._yahoo_backoff_until:
            return None
        wait = market._yahoo_next_at - time.monotonic()
        if wait > 0:
            time.sleep(wait)
        market._yahoo_next_at = time.monotonic() + market.YAHOO_MIN_INTERVAL_SEC
        answered = call()
        if getattr(answered, "status_code", 0) == 429:
            market._yahoo_backoff_until = time.monotonic() + market.YAHOO_BACKOFF_SEC
            return None
        return answered


def float_shares(symbol, exchange, currency, name="", ssl_context=None):
    """What a short position is measured against: the shares actually available to trade.

    For a company that is the free float, which Yahoo publishes for both markets under the
    same symbol forms the app's own quotes use, and it is never swapped for the shares in
    issue — those include what insiders hold and would not be the same percentage twice. A
    fund is the one instrument where the two are the same thing: its units are created and
    redeemed on demand and none are held back, so the units in issue are the units there are
    to trade, and where no float is published for one its unit count stands in its place.
    None where neither is published, and the tile then says so."""
    sym = _s(symbol).strip().upper()
    key = "%s|%s" % (sym, _s(exchange).strip().upper())
    with _lock:
        held = _shares.get(key)
        if held and time.time() - held["at"] < (FLOAT_HOURS * 3600 if held["float"] else FLOAT_MISS_MIN * 60):
            return held["float"]
    count, fund = None, exposure.is_fund(name, sym)
    # the symbol forms follow the market the venue already settled on, not the currency the
    # row happens to carry: a watchlist row keeps none, and the forms then default to Canada,
    # so a Nasdaq listing was asked for as a Toronto one and answered with nothing
    where = market_of(sym, exchange, currency)
    ccy = _s(currency).strip() or ("USD" if where == "us" else "CAD")
    session, crumb = _yahoo_session()
    if session:
        for form in (market.yahoo_forms({"symbol": sym, "exchange": exchange, "currency": ccy}) or [market.tmx_symbol(sym)]):
            try:
                answered = _yahoo_paced(lambda: session.get(YAHOO_STATS_URL % (form, crumb), timeout=market.TIMEOUT_SEC))
                if answered is None or answered.status_code != 200:
                    continue
                stats = (((answered.json().get("quoteSummary") or {}).get("result") or [{}])[0] or {}).get("defaultKeyStatistics") or {}
                pick = lambda field: _num((stats.get(field) or {}).get("raw") if isinstance(stats.get(field), dict) else stats.get(field))
                count = pick("floatShares") or (pick("sharesOutstanding") if fund else None)
                if count:
                    break
            except Exception as e:
                sys.stderr.write("bagholder shorts: %s float from yahoo failed: %s\n" % (form, e))
    if not count and fund:
        count = _fund_units(sym, exchange, ccy, ssl_context)
    count = count or None
    with _lock:
        _shares[key] = {"float": count, "at": time.time()}
    return count


def _venue_fits(code, exchange):
    """Whether a row's venue is the listing's. A symbol appears once in each Canadian
    file, so this confirms the row rather than choosing between rows."""
    ex = _s(exchange).strip().upper()
    return not ex or ex in CA_VENUES.get(_s(code).strip().upper(), ())


# --- one listing -------------------------------------------------------------------

def us_position(symbol, ssl_context=None, now=None):
    """The newest settlement FINRA has for a US listing, and the date of the one before it."""
    now = now or datetime.now(timezone.utc)
    body = {"limit": 20,
            "compareFilters": [{"fieldName": "symbolCode", "fieldValue": _s(symbol).strip().upper(), "compareType": "EQUAL"}],
            "dateRangeFilters": [{"fieldName": "settlementDate", "startDate": (now - timedelta(days=150)).strftime("%Y-%m-%d"),
                                  "endDate": now.strftime("%Y-%m-%d")}]}
    try:
        answered = market._post_json(US_POSITION_URL, body, ssl_context, {"Accept": "application/json"})
    except Exception as e:
        sys.stderr.write("bagholder shorts: %s position from finra failed: %s\n" % (symbol, e))
        return {}
    rows = [r for r in answered or [] if isinstance(r, dict) and r.get("settlementDate")]
    if not rows:
        return {}
    rows.sort(key=lambda x: _s(x.get("settlementDate")), reverse=True)
    r = rows[0]
    return {"asOf": _s(r.get("settlementDate"))[:10], "shares": _num(r.get("currentShortPositionQuantity")),
            "previous": _num(r.get("previousShortPositionQuantity")), "change": _num(r.get("changePreviousNumber")),
            "previousOf": _s(rows[1].get("settlementDate"))[:10] if len(rows) > 1 else "",
            "averageVolume": _num(r.get("averageDailyVolumeQuantity")),
            # every settlement FINRA answered with, oldest first: the run of reports costs
            # nothing extra here, since they arrive in the same answer as the newest one
            "series": [{"date": _s(x.get("settlementDate"))[:10], "shares": _num(x.get("currentShortPositionQuantity"))}
                       for x in sorted(rows, key=lambda y: _s(y.get("settlementDate")))
                       if _num(x.get("currentShortPositionQuantity")) is not None]}


def us_volume(symbol, ssl_context=None, now=None):
    """The last trading day's short volume for a US listing."""
    held = _table("us_volume", _us_volume_file, ssl_context, now or datetime.now(timezone.utc))
    row = held["rows"].get(_s(symbol).strip().upper())
    if not row:
        return {}
    pct = (row["shortVolume"] / row["totalVolume"] * 100) if row.get("totalVolume") else None
    return {"volumeOf": held["key"], "volumeSpan": "day", "shortVolume": row["shortVolume"],
            "totalVolume": row["totalVolume"], "volumePct": pct}


def ca_position(symbol, exchange="", ssl_context=None, now=None):
    held = _table("ca_position", _ca_position_file, ssl_context, now or datetime.now(timezone.utc))
    row = held["rows"].get(_s(symbol).strip().upper())
    if not row or not _venue_fits(row.get("venue"), exchange):
        return {}
    shares, change = row["shares"], row.get("change")
    # the report before this one is the previous reporting date, which is what the change is against
    earlier = [d.isoformat() for d in position_dates(now.date() if now else datetime.now(timezone.utc).date()) if d.isoformat() < held["key"]]
    return {"asOf": held["key"], "shares": shares, "change": change,
            "previous": (shares - change) if change is not None else None,
            "previousOf": earlier[0] if earlier else "",
            # the report names the venue and the issuer: what a listing the app knows nothing about is
            "venue": _s(row.get("venue")).strip().upper(), "issuer": _s(row.get("name")).strip()}


def ca_volume(symbol, exchange="", currency="", ssl_context=None, now=None):
    """The short part of a Canadian listing's trading over the report's period. A listing the
    report does not carry was not sold short in it — the report has no zero rows — so its short
    volume is none of its trading rather than unknown, and what it did trade comes from the
    exchange so days to cover still has a denominator."""
    held = _table("ca_volume", _ca_volume_file, ssl_context, now or datetime.now(timezone.utc))
    if not held["key"]:
        return {}
    row = held["rows"].get(_s(symbol).strip().upper())
    if row and not _venue_fits(row.get("venue"), exchange):
        return {}
    if not row:
        traded = ca_traded(symbol, exchange, currency, held["key"], ssl_context)
        if not traded:
            return {}
        return {"volumeOf": held["key"], "volumeSpan": "period", "shortVolume": 0.0,
                "totalVolume": traded, "volumePct": 0.0}
    return {"volumeOf": held["key"], "volumeSpan": "period", "shortVolume": row["shortVolume"],
            "totalVolume": row.get("totalVolume"), "volumePct": row.get("volumePct")}


def average_volume(rec, now=None):
    """The average daily volume in the listing's own market, over the period its short report
    covers. It cannot come from the app's own bars: those are stored under the bare ticker, so
    a company listed on both markets has one set of them and days to cover for its US listing
    would be measured against Canadian trading. Each regulator reports the volume for its own
    market instead — FINRA publishes the average itself; Canada's report gives the period's
    total, divided here by the days the Canadian market actually traded, counted from the index
    series the app already keeps so a holiday is not counted as a day of trading."""
    if rec.get("market") == "us":
        return rec.get("averageVolume")
    total, span = rec.get("totalVolume"), _s(rec.get("volumeOf"))
    if not total or "/" not in span:
        return None
    start, end = span.split("/", 1)
    days = store.benchmark_days("TSX", start, end)
    return (total / days) if days else None


def days_to_cover(rec, now=None):
    """The position against the app's own average daily volume over the last sessions it
    has bars for. One calculation on both markets, rather than each regulator's own
    arithmetic where it happens to publish some. A listing with no bars stored yet gets
    them through the app's own history, the same daily bars its chart is about to ask
    for; None only when even that answers nothing."""
    shares, average = rec.get("shares"), average_volume(rec, now)
    return round(shares / average, 1) if shares and average else None


def for_listing(symbol, exchange="", currency="", ssl_context=None, now=None, trend=False, name=""):
    """Everything published about one listing's short selling, {} where nothing is."""
    sym = _s(symbol).strip().upper()
    now = now or datetime.now(timezone.utc)
    where = market_of(sym, exchange, currency)
    if not where:
        return {}
    if where == "us":
        rec = dict(us_position(sym, ssl_context, now))
        rec.update(us_volume(sym, ssl_context, now))
    else:
        rec = dict(ca_position(sym, exchange, ssl_context, now))
        rec.update(ca_volume(sym, exchange, currency, ssl_context, now))
    if where == "ca" and trend:
        rec["series"] = ca_series(sym, exchange, rec.get("asOf") or "", ssl_context, now)
    # the record names its own listing, as a stored one does: a ticker read on the spot for the
    # ranked list's box is not in the store yet, and without this its row had no exchange. A
    # listing the app knew no venue for takes the one the regulator's own report gives it,
    # and the issuer's name with it, so a searched row reads like every other row.
    venue, issuer = _s(rec.pop("venue", "")).strip().upper(), _s(rec.pop("issuer", "")).strip()
    rec.update({"symbol": sym, "exchange": _s(exchange).strip().upper() or CA_VENUE_NAMES.get(venue, ""), "market": where,
                "source": "FINRA" if where == "us" else "CIRO"})
    if issuer:
        rec["name"] = issuer
    # a listing the book does not carry is asked for under its ticker, and a ticker is not a name:
    # the issuer the report names is what tells a fund from a company, and so what its short
    # position is measured against — the units in issue rather than a float nobody publishes
    floated = float_shares(sym, exchange, currency, _s(name).strip() or issuer, ssl_context)
    rec["float"] = floated
    rec["ofFloat"] = (rec["shares"] / floated * 100) if floated and rec.get("shares") else None
    rec["averageVolume"] = average_volume(rec, now)
    rec["daysToCover"] = days_to_cover(rec, now)
    return rec
