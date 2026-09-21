"""The fear and greed indexes as their publishers give them: CNN's for the US stock market,
the meter the media carries, and alternative.me's for crypto.

Nothing here is computed from the market. A reading is the publisher's own score, its own
rating and, where it publishes them, its own indicators; a score a publisher gives without a
rating (the readings it compares today against) is named on that same publisher's scale,
which is the scale the score was made on."""
from __future__ import annotations

import json
import sys
from datetime import datetime, timezone

import market

STOCK_URL = "https://production.dataviz.cnn.io/index/fearandgreed/graphdata"
CRYPTO_URL = "https://api.alternative.me/fng/?limit=%d"
DAYS = 366                    # a year of daily readings, which is what both publish
STOCK_HEADERS = {"User-Agent": market.UA, "Accept": "application/json",
                 "Origin": "https://www.cnn.com", "Referer": "https://www.cnn.com/"}
CRYPTO_HEADERS = {"User-Agent": market.UA, "Accept": "application/json"}
INDEXES = ("stocks", "crypto")
SOURCES = {"stocks": "CNN", "crypto": "Alternative.me"}
# CNN's seven indicators under CNN's own names. Its answer carries two forms of the momentum
# and volatility ones; these are the averages its own page names, the S&P 500's 125-day and
# the VIX's 50-day.
PARTS = (("market_momentum_sp125", "Market momentum"),
         ("stock_price_strength", "Stock price strength"),
         ("stock_price_breadth", "Stock price breadth"),
         ("put_call_options", "Put and call options"),
         ("market_volatility_vix_50", "Market volatility"),
         ("junk_bond_demand", "Junk bond demand"),
         ("safe_haven_demand", "Safe haven demand"))
# the scale both publishers name their own scores on
BANDS = ((25, "Extreme fear"), (45, "Fear"), (56, "Neutral"), (76, "Greed"))


def _s(v):
    return "" if v is None else str(v)


def _num(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def band(score):
    """What a score is called on the publishers' own scale: under 25 extreme fear, under 45
    fear, 45 to 55 neutral, 56 to 75 greed, 76 and over extreme greed."""
    n = _num(score)
    if n is None:
        return ""
    for edge, name in BANDS:
        if n < edge:
            return name
    return "Extreme greed"


def rating(given, score):
    """The publisher's own word for the reading where it gives one — `extreme fear` reads as
    `Extreme fear` — and its own scale's word where it gives only a number."""
    text = _s(given).strip()
    return (text[:1].upper() + text[1:].lower()) if text else band(score)


def _day(ms):
    """A point's day, from the milliseconds both publishers stamp their history with."""
    n = _num(ms)
    if n is None:
        return ""
    return datetime.fromtimestamp(n / 1000.0, timezone.utc).strftime("%Y-%m-%d")


def _moment(text):
    """CNN stamps the live reading with an offset time; kept as the app writes times."""
    try:
        return datetime.fromisoformat(_s(text)).astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    except ValueError:
        return ""


def _reading(label, score):
    n = _num(score)
    return {"label": label, "score": round(n, 1), "rating": band(n)} if n is not None else None


def parse_stocks(data):
    """CNN's answer: the reading now, the readings it compares itself against, its seven
    indicators, and a year of daily readings. {} when it carries no score."""
    fg = (data or {}).get("fear_and_greed") or {}
    score = _num(fg.get("score"))
    if score is None:
        return {}
    earlier = [_reading("Previous close", fg.get("previous_close")), _reading("A week ago", fg.get("previous_1_week")),
               _reading("A month ago", fg.get("previous_1_month")), _reading("A year ago", fg.get("previous_1_year"))]
    parts = []
    for key, name in PARTS:
        part = (data or {}).get(key) or {}
        value = _num(part.get("score"))
        if value is not None:
            parts.append({"name": name, "score": round(value, 1), "rating": rating(part.get("rating"), value)})
    series = []
    for point in ((data or {}).get("fear_and_greed_historical") or {}).get("data") or []:
        day, value = _day(point.get("x")), _num(point.get("y"))
        if day and value is not None:
            series.append({"date": day, "score": round(value, 1)})
    return {"index": "stocks", "source": SOURCES["stocks"], "score": round(score, 1),
            "rating": rating(fg.get("rating"), score), "asOf": _moment(fg.get("timestamp")),
            "previous": [r for r in earlier if r], "parts": parts, "series": sorted(series, key=lambda r: r["date"])}


def parse_crypto(data):
    """Alternative.me's answer: one reading a day, newest first. The readings it is compared
    against are its own earlier days; it publishes no indicators under the index."""
    rows = []
    for row in (data or {}).get("data") or []:
        day, value = _day(_num(row.get("timestamp")) * 1000 if _num(row.get("timestamp")) is not None else None), _num(row.get("value"))
        if day and value is not None:
            rows.append({"date": day, "score": round(value, 1), "rating": rating(row.get("value_classification"), value)})
    if not rows:
        return {}
    now = rows[0]
    at = lambda i, label: ({"label": label, "score": rows[i]["score"], "rating": rows[i]["rating"]} if len(rows) > i else None)
    earlier = [at(1, "Yesterday"), at(7, "A week ago"), at(30, "A month ago"), at(365, "A year ago")]
    return {"index": "crypto", "source": SOURCES["crypto"], "score": now["score"], "rating": now["rating"],
            "asOf": now["date"] + "T00:00:00Z", "previous": [r for r in earlier if r], "parts": [],
            "series": sorted(({"date": r["date"], "score": r["score"]} for r in rows), key=lambda r: r["date"])}


def read(index, ssl_context=None):
    """One index as its publisher gives it now, {} where it did not answer."""
    which = _s(index).strip().lower()
    try:
        if which == "stocks":
            return parse_stocks(json.loads(market._get_text(STOCK_URL, ssl_context, headers=STOCK_HEADERS)))
        if which == "crypto":
            return parse_crypto(json.loads(market._get_text(CRYPTO_URL % DAYS, ssl_context, headers=CRYPTO_HEADERS)))
    except Exception as e:
        sys.stderr.write("bagholder fear: %s from %s failed: %s\n" % (which, SOURCES.get(which, "its publisher"), e))
    return {}
