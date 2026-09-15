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
import sys
import threading
import time
from datetime import date, datetime, timedelta, timezone

import instruments
import market
import model
import store
import xls

US_POSITION_URL = "https://api.finra.org/data/group/otcMarket/name/consolidatedShortInterest"
US_VOLUME_URL = "https://cdn.finra.org/equity/regsho/daily/CNMSshvol%s.txt"
CA_POSITION_URL = "https://www.ciro.ca/sites/default/files/epubs/CSPR/%s_CSPR_Report.xls"
CA_VOLUME_URL = "https://www.ciro.ca/sites/default/files/epubs/SSALE/%s-%s_ShortSaleTradingSummaryReport.csv"
# what the Canadian files call each venue, against what the app calls it
CA_VENUES = {"TSX": ("TSX",), "TSXV": ("TSX-V", "TSXV"), "CSE": ("CSE",), "AQL": ("CBOE CANADA", "NEO")}
FILE_HOURS = 6           # how often a whole-market file is looked for again
TRIES = 6                # how many report dates back to try before giving up
HEADERS = {"User-Agent": market.UA, "Accept": "*/*"}

_files = {}
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
            "averageVolume": _num(r.get("averageDailyVolumeQuantity"))}


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
            "previousOf": earlier[0] if earlier else ""}


def ca_volume(symbol, exchange="", ssl_context=None, now=None):
    held = _table("ca_volume", _ca_volume_file, ssl_context, now or datetime.now(timezone.utc))
    row = held["rows"].get(_s(symbol).strip().upper())
    if not row or not _venue_fits(row.get("venue"), exchange):
        return {}
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


def for_listing(symbol, exchange="", currency="", ssl_context=None, now=None):
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
        rec.update(ca_volume(sym, exchange, ssl_context, now))
    rec.update({"symbol": sym, "market": where, "source": "FINRA" if where == "us" else "CIRO"})
    rec["averageVolume"] = average_volume(rec, now)
    rec["daysToCover"] = days_to_cover(rec, now)
    return rec
