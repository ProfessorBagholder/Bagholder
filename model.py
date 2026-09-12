"""Derived trading model, computed in Python from the SQLite store.

Everything the v2 UI shows comes from here so that one list of trades feeds
every tile, table and chart, and so the numbers can be tested.

Pipeline
    activities  -> normalize_activities  (crypto, options, stock-dividend notices)
                -> match_fifo            (FIFO lots per account+symbol+currency,
                                          round-trip ids, option rolls)
                -> apply_fx              (P&L in CAD on the fill dates)
                -> build_trades          (round trips + saved manual groups)
                -> build_positions       (open lots rolled up per symbol+account)
                -> build_cashflow        (dividends, interest, withholding tax)
    nav_history -> equity series, yearly time-weighted returns, drawdown
    build_view(filters) applies one filter object to all of the above and
    computes the KPIs, monthly buckets, by-symbol and grade tables from the
    same filtered list.

Currency: per-trade numbers are native (USD trades stay in USD). Anything that
adds trades together (KPIs, monthly, by symbol, grade buckets, cashflow tiles)
uses the CAD value converted on the fill date with the Bank of Canada rate.

A "trade" is a round trip: the position goes from flat to open and back to
flat. Partial exits are legs of the same trade. A trade's id is stable from the first fill, so journal entries survive
later exits; whatever is still held shows under positions.
"""

from __future__ import annotations

import json
import functools
import re
import threading
from datetime import date, datetime, timedelta, timezone

import exposure
import instruments
import market
import news
import store

EPS = 1e-10
FX_FALLBACK = 1.35
GRADES = ("A", "B", "C", "F")
TIME_TZ = store.ACTIVITY_PULL_TZ
MONTHS = ("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")
KINDS = ("Shares", "Options", "Crypto", "Futures")

_SPACE_RE = re.compile(r"[\s  -​  　]+")


# --------------------------------------------------------------------------
# small helpers
# --------------------------------------------------------------------------


def _s(v):
    return "" if v is None else str(v)


def _num(v, default=0.0):
    if v is None or v == "":
        return default
    try:
        f = float(v)
    except (TypeError, ValueError):
        return default
    if f != f:  # NaN
        return default
    return f


# Symbols and account names are normalised in the inner loops of the FIFO
# match: half a million calls over a few dozen distinct strings for one book.
# The results depend on nothing but the string, so they are remembered.
@functools.lru_cache(maxsize=8192)
def _compact(s):
    return re.sub(r"[\s_\-]+", "", s.strip().upper())


def compact(s):
    return _compact(_s(s))


@functools.lru_cache(maxsize=8192)
def _norm_account_name(s):
    return _SPACE_RE.sub(" ", s).strip()


def norm_account_name(s):
    """Nicknames mix ASCII and en-space separators; equality filters need one."""
    return _norm_account_name(_s(s))


@functools.lru_cache(maxsize=8192)
def _is_option_symbol(symbol):
    u = _SPACE_RE.sub(" ", symbol.strip().upper())
    if not u:
        return False
    if re.search(r"\b(PUT|CALL)\b", u) or re.search(r"\s[CP]$", u):
        return True
    if re.match(r"^[A-Z][A-Z0-9.]{0,9} \d{6}[CP]\d+", u):
        return True
    return False


def is_option_symbol(symbol):
    return _is_option_symbol(_s(symbol))


def underlying_symbol(symbol):
    s = _s(symbol).strip()
    if not s:
        return "—"
    u = _SPACE_RE.sub(" ", s.upper())
    if re.search(r"\b(PUT|CALL)\b", u) or re.search(r"\s[CP]$", u):
        return u.split(" ")[0] or s
    m = re.match(r"^([A-Z][A-Z0-9.]{0,9}) \d{6}[CP]\d+", u)
    if m:
        return m.group(1)
    m = re.match(r"^([A-Z][A-Z0-9.]{0,9}) \d{1,2}[A-Z]{3}\d{2}\b", u)
    if m:
        return m.group(1)
    return s


def option_multiplier(symbol):
    return 100 if is_option_symbol(symbol) else 1


def option_right(symbol):
    u = _SPACE_RE.sub(" ", _s(symbol).strip().upper())
    if u.endswith(" PUT") or u.endswith(" P") or re.search(r" \d{6}P\d+", u):
        return "PUT"
    return "CALL"


def is_multileg(a):
    return "MULTILEG" in compact(a.get("rawType"))


def roll_key(a):
    return (fifo_account(a), underlying_symbol(a.get("symbol")), option_right(a.get("symbol")))


def days_between(a, b):
    try:
        da = date.fromisoformat(_s(a)[:10])
        db = date.fromisoformat(_s(b)[:10])
    except ValueError:
        return 0
    return max(0, (db - da).days)


def shift_date(iso, days):
    try:
        return (date.fromisoformat(_s(iso)[:10]) + timedelta(days=days)).isoformat()
    except ValueError:
        return _s(iso)[:10]


def today_local():
    return datetime.now(TIME_TZ).date().isoformat()


def when_parts(occurred):
    """ISO instant -> (YYYY-MM-DD, HH:MM) in the app's local time zone."""
    s = _s(occurred).strip()
    if not s:
        return "", ""
    if "T" not in s:
        return s[:10], ""
    try:
        if s.endswith("Z"):
            s = s[:-1] + "+00:00"
        dt = datetime.fromisoformat(s)
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        loc = dt.astimezone(TIME_TZ)
        return loc.date().isoformat(), loc.strftime("%H:%M")
    except ValueError:
        return s[:10], ""


def fmt8(v):
    return "%.8f" % _num(v)


# --------------------------------------------------------------------------
# activity normalization
# --------------------------------------------------------------------------


def is_crypto_activity(a):
    return compact(a.get("rawType")).startswith("CRYPTO") or compact(a.get("activityType")).startswith("CRYPTO")


def kind_of(a):
    if a.get("kind") in KINDS:
        return a["kind"]
    if is_crypto_activity(a):
        return "Crypto"
    if is_option_symbol(a.get("symbol")):
        return "Options"
    return "Shares"


def is_intentional_open(a):
    fields = (compact(a.get("activityType")), compact(a.get("activitySubType")))
    if any("TOOPEN" in f for f in fields):
        return True
    return any(f in ("STO", "BTO") for f in fields)


def is_close_only(a):
    fields = (compact(a.get("activityType")), compact(a.get("activitySubType")))
    if any("TOCLOSE" in f or f in ("BTC", "STC") for f in fields):
        return True
    return any("EXPIR" in f or "ASSIGN" in f or "EXERCISE" in f for f in fields)


def opening_direction(a, side):
    if side == "BUY":
        return None if is_close_only(a) else "LONG"
    if side == "SELL":
        if is_option_symbol(a.get("symbol")):
            return None if is_close_only(a) else "SHORT"
        if is_intentional_open(a):
            return "SHORT"
        if a.get("kind") == "Crypto":
            return None
        return None
    return None


def normalize_activity(activity):
    """Copy of the row with crypto and option events expressed as trade fills."""
    a = dict(activity)
    a["accountType"] = norm_account_name(a.get("accountType"))
    rt = compact(a.get("rawType"))
    at = compact(a.get("activityType"))
    cash = _num(a.get("netCashAmount"))
    qty = abs(_num(a.get("quantity")))
    a["flags"] = []

    if rt == "CRYPTOBUY" or at == "CRYPTOBUY":
        a.update(category="trade", activityType="Trade", activitySubType="BUY", kind="Crypto")
        a["quantity"] = qty
        a["netCashAmount"] = -abs(cash)
        return a
    if rt == "CRYPTOSELL" or at == "CRYPTOSELL":
        a.update(category="trade", activityType="Trade", activitySubType="SELL", kind="Crypto")
        a["quantity"] = -qty
        a["netCashAmount"] = abs(cash)
        return a
    if rt == "CRYPTOTRANSFER" or at == "CRYPTOTRANSFER":
        sub = compact(a.get("activitySubType"))
        a.update(category="trade", activityType="Trade", kind="Crypto")
        a["flags"].append("transfer")
        if "OUT" in sub or cash < 0:
            a["activitySubType"] = "SELL"
            a["flags"].append("transfer-out")
            a["quantity"] = -qty
            a["netCashAmount"] = abs(cash)
        else:
            a["activitySubType"] = "BUY"
            a["quantity"] = qty
            a["netCashAmount"] = -abs(cash)
        return a
    if rt == "CRYPTOSTAKINGREWARD" or at == "CRYPTOSTAKINGREWARD":
        a.update(category="trade", activityType="Trade", activitySubType="BUY", kind="Crypto")
        a["flags"].append("reward")
        a["quantity"] = qty
        a["unitPrice"] = 0.0
        a["netCashAmount"] = 0.0
        return a
    if rt.startswith("CRYPTO"):
        a["category"] = "other"
        a["kind"] = "Crypto"
        return a

    if at == "STKDIS" and rt == "DIVIDEND" and abs(cash) < EPS:
        # A distribution posted in units with no cash is a pending notice,
        # not a share delivery: Wealthsimple's balance does not grow by it.
        a["category"] = "other"
        a["flags"].append("pending-distribution")
        return a

    raw = rt + at
    if "MULTILEG" in raw:
        a["category"] = "trade"
        if cash < 0 or compact(a.get("direction")) == "DEBIT":
            a["activityType"] = "OPTIONS_BUY"
            a["activitySubType"] = "BUYTOCLOSE"
        else:
            a["activityType"] = "OPTIONS_SELL"
            a["activitySubType"] = "SELLTOOPEN"
    elif "EXPIR" in raw or "ASSIGN" in raw or "EXERCISE" in raw:
        a["category"] = "option_event"
        if "ASSIGN" in raw:
            a["activityType"] = "ASSIGN"
            a["activitySubType"] = "BUYTOCLOSE"
            a["unitPrice"] = 0.0
        elif "SHORTEXPIR" in raw:
            a["activityType"] = "EXPIR"
            a["activitySubType"] = "BUY"
        elif "EXPIR" in raw:
            a["activityType"] = "EXPIR"
            a["activitySubType"] = "SELL"
        else:
            a["activityType"] = "EXERCISE"
            a["activitySubType"] = "SELL"
        if "ASSIGN" in raw or abs(cash) < 1e-12:
            a["unitPrice"] = 0.0
        if qty > 0:
            a["quantity"] = -qty if a["activitySubType"] == "SELL" else qty
    a["kind"] = kind_of(a)
    return a


def normalize_activities(activities):
    return [normalize_activity(a) for a in activities or []]


def fold_stkdis(activities):
    """Net +N/-N name-change rows on one day; leftover +N opens at $0."""
    rest = []
    groups = {}
    order = []
    for a in activities:
        if compact(a.get("activityType")) != "STKDIS":
            rest.append(a)
            continue
        k = (a.get("symbol"), a.get("transactionDate"), a.get("currency"))
        if k not in groups:
            groups[k] = {"pos": 0.0, "neg": 0.0, "sample": a}
            order.append(k)
        g = groups[k]
        q = _num(a.get("quantity"))
        if a.get("activitySubType") == "SELL" or q < 0:
            g["neg"] += abs(q)
        else:
            g["pos"] += abs(q)
    for k in order:
        g = groups[k]
        net = g["pos"] - g["neg"]
        if net > EPS:
            a = dict(g["sample"])
            a.update(quantity=net, activitySubType="BUY", unitPrice=0.0, netCashAmount=0.0, category="trade")
            rest.append(a)
    return rest


def split_markers(activities):
    """Wealthsimple posts a share split as a CORPORATE_ACTION with quantity 0
    and no ratio. Infer the ratio from the fill prices on either side and
    return {(account, symbol, currency, date): factor}, where lot quantities
    are multiplied by factor and prices divided by it (1/5 for a 1-for-5
    reverse split, 4 for a 4-for-1 split)."""
    out = {}
    by_book = {}
    for a in activities:
        if a.get("category") not in ("trade", "option_event") or not a.get("symbol"):
            continue
        by_book.setdefault((fifo_account(a), _s(a.get("symbol"))), []).append(a)
    for a in activities:
        if compact(a.get("activityType")) != "STKDIS" or compact(a.get("rawType")) != "CORPORATEACTION":
            continue
        if abs(_num(a.get("quantity"))) > EPS:
            continue
        day = _s(a.get("transactionDate"))
        # the marker's currency does not always match the fills'; key on account+symbol
        key = (fifo_account(a), _s(a.get("symbol")))
        priced = sorted(
            (x for x in by_book.get(key, []) if _num(x.get("unitPrice")) > 0 and compact(x.get("activityType")) != "STKDIS"),
            key=lambda x: (_s(x.get("transactionDate")), _s(x.get("occurredAt"))),
        )
        before = [_num(x.get("unitPrice")) for x in priced if _s(x.get("transactionDate")) < day][-3:]
        after = [_num(x.get("unitPrice")) for x in priced if _s(x.get("transactionDate")) >= day][:3]
        if not before or not after:
            continue
        before.sort()
        after.sort()
        pre = before[len(before) // 2]
        post = after[len(after) // 2]
        if not pre > 0 or not post > 0:
            continue
        ratio = post / pre
        if ratio >= 1.5:
            n = round(ratio)
            factor = 1.0 / n
        elif ratio <= 1 / 1.5:
            n = round(1 / ratio)
            factor = float(n)
        else:
            continue
        if n < 2 or abs(ratio - (1 / factor)) / (1 / factor) > 0.35:
            continue
        out[(fifo_account(a), _s(a.get("symbol")), day)] = factor
    return out


def fifo_account(a):
    nick = norm_account_name(a.get("accountType"))
    if nick:
        return nick
    if a.get("fifoId"):
        return _s(a.get("fifoId"))
    return _s(a.get("accountId"))


def book_key(a):
    return fifo_account(a) + "::" + _s(a.get("symbol")) + "::" + _s(a.get("currency"))


_REMOVAL_RE = re.compile(r"CODECHANGE|SYMBOLCHANGE|TICKERCHANGE|LISTINGSTATUS|SECURITYSWAP")


def replacement_index(activities):
    """Per (account, symbol, currency): the first date the ticker was removed
    (code change / STKDIS out) and the dates of real trades, computed once so
    ticker_was_replaced is a lookup instead of a scan."""
    removed = {}
    trades = {}
    for a in activities:
        key = (fifo_account(a), _s(a.get("symbol")), _s(a.get("currency")))
        t = compact(a.get("activityType"))
        d = _s(a.get("transactionDate"))
        if t == "STKDIS":
            sub = compact(a.get("activitySubType"))
            if sub == "SELL" or _num(a.get("quantity")) < 0:
                if d and (key not in removed or d < removed[key]):
                    removed[key] = d
            continue
        raw = compact(a.get("rawType")) + compact(a.get("aftType"))
        if _REMOVAL_RE.search(raw):
            if d and (key not in removed or d < removed[key]):
                removed[key] = d
        if a.get("category") in ("trade", "option_event") and store.trade_side(a):
            trades.setdefault(key, []).append(d)
    return {"removed": removed, "trades": trades}


def ticker_was_replaced(index, account, symbol, currency, by_date):
    key = (_s(account), _s(symbol), _s(currency))
    removed_on = index["removed"].get(key)
    if not removed_on or removed_on > by_date:
        return False
    return not any(d > removed_on for d in index["trades"].get(key, []))


# --------------------------------------------------------------------------
# option quantity inference (WS multileg rows often carry qty 0)
# --------------------------------------------------------------------------


def _set_fill_side(f, side, sub):
    f["side"] = side
    f["a"]["activitySubType"] = sub
    q = abs(_num(f["a"].get("quantity")))
    if q > 0:
        f["a"]["quantity"] = -q if side == "SELL" else q


def _resolve_option_fill_side(f, rem):
    a = f["a"]
    raw = compact(a.get("rawType")) + compact(a.get("activityType"))
    expirish = "EXPIR" in raw or "ASSIGN" in raw or "EXERCISE" in raw
    if expirish:
        if "ASSIGN" in raw or "SHORTEXPIR" in raw:
            _set_fill_side(f, "BUY", "BUYTOCLOSE" if "ASSIGN" in raw else "BUY")
        elif "EXPIR" in raw and "SHORT" not in raw:
            _set_fill_side(f, "SELL", "SELL")
        elif f["side"] == "BUY" and rem["LONG"] > EPS and rem["SHORT"] <= EPS:
            _set_fill_side(f, "SELL", "SELL")
        elif f["side"] == "SELL" and rem["SHORT"] > EPS and rem["LONG"] <= EPS:
            _set_fill_side(f, "BUY", "BUY")
        return
    if not ("MULTILEG" in raw or is_close_only(a)):
        return
    if f["side"] == "BUY":
        a["activitySubType"] = "BUYTOCLOSE" if rem["SHORT"] > EPS else "BUYTOOPEN"
    elif f["side"] == "SELL":
        a["activitySubType"] = "SELLTOCLOSE" if rem["LONG"] > EPS else "SELLTOOPEN"


def _is_clean_option_qty(cash, qty):
    if not qty > 0:
        return False
    px = abs(cash) / (qty * 100.0)
    if px < 0:
        return False
    if abs(px * 100 - round(px * 100)) < 1e-6:
        return True
    if abs(px * 10000 - round(px * 10000)) < 1e-4:
        return True
    return False


def _infer_standalone_option_qty(cash):
    abs_cash = abs(cash)
    if not abs_cash > 0:
        return 0
    max_qty = min(10000, max(1, int(round(abs_cash))))
    for q in range(1, max_qty + 1):
        if _is_clean_option_qty(abs_cash, q):
            return q
    return 1


def infer_zero_qty_option_fills(fills):
    remaining = {}

    def rem_of(a):
        k = book_key(a)
        if k not in remaining:
            remaining[k] = {"LONG": 0.0, "SHORT": 0.0}
        return remaining[k]

    zeros_by_book = {}
    pools = {}

    def pool_of(a):
        k = roll_key(a)
        if k not in pools:
            pools[k] = {"LONG": 0.0, "SHORT": 0.0}
        return pools[k]

    for i, f in enumerate(fills):
        a = f["a"]
        qty = abs(_num(a.get("quantity")))
        cash = _num(a.get("netCashAmount"))
        if not is_option_symbol(a.get("symbol")) or not f["side"]:
            continue
        if qty == 0 and abs(cash) > 1e-9:
            zeros_by_book.setdefault(book_key(a), []).append(i)

    for i, f in enumerate(fills):
        a = f["a"]
        if not f["side"]:
            continue
        rem = rem_of(a)
        if is_option_symbol(a.get("symbol")) and is_multileg(a):
            # A roll: this row closes what the contract holds (or what an earlier
            # roll carried forward), and the same quantity moves to the next contract.
            pool = pool_of(a)
            direction = "SHORT" if rem["SHORT"] > EPS else "LONG" if rem["LONG"] > EPS else ("SHORT" if pool["SHORT"] >= pool["LONG"] else "LONG")
            open_sz = rem[direction] + pool[direction]
            qty = abs(_num(a.get("quantity")))
            cash = _num(a.get("netCashAmount"))
            if qty == 0:
                k = book_key(a)
                upcoming = len([j for j in zeros_by_book.get(k, []) if j > i])
                if rem[direction] > EPS and upcoming == 0:
                    qty = rem[direction]
                elif open_sz > EPS:
                    picked = 0
                    cap = max(1, int(open_sz + 1e-9))
                    for q in range(1, cap + 1):
                        if _is_clean_option_qty(cash, q):
                            picked = q
                            break
                    qty = picked or _infer_standalone_option_qty(cash)
                    if qty > open_sz:
                        qty = open_sz
                else:
                    qty = _infer_standalone_option_qty(cash)
                a["unitPrice"] = abs(cash) / (qty * 100.0) if qty > 0 else 0.0
            f["side"] = "BUY" if direction == "SHORT" else "SELL"
            a["activitySubType"] = "BUYTOCLOSE" if direction == "SHORT" else "SELLTOCLOSE"
            a["quantity"] = qty if direction == "SHORT" else -qty
            f["qty"] = qty
            f["rollDirection"] = direction
            closed = min(qty, rem[direction])
            rem[direction] -= closed
            pool[direction] -= min(qty - closed, pool[direction])
            if closed > EPS or qty > EPS:
                pool[direction] += qty
            continue
        if is_option_symbol(a.get("symbol")):
            _resolve_option_fill_side(f, rem)
        qty = abs(_num(a.get("quantity")))
        cash = _num(a.get("netCashAmount"))
        if is_option_symbol(a.get("symbol")) and qty == 0:
            closing_dir = "SHORT" if f["side"] == "BUY" else "LONG"
            open_sz = rem.get(closing_dir, 0.0)
            if abs(cash) > 1e-9:
                k = book_key(a)
                upcoming = len([j for j in zeros_by_book.get(k, []) if j > i])
                if open_sz > 0 and upcoming == 0:
                    qty = open_sz
                elif open_sz > 0:
                    picked = 0
                    cap = max(1, int(open_sz + 1e-9))
                    for q in range(1, cap + 1):
                        if _is_clean_option_qty(cash, q):
                            picked = q
                            break
                    qty = picked or _infer_standalone_option_qty(cash)
                    if qty > open_sz:
                        qty = open_sz
                else:
                    qty = _infer_standalone_option_qty(cash)
                a["unitPrice"] = abs(cash) / (qty * 100.0) if qty > 0 else 0.0
            elif open_sz > 0 and is_close_only(a):
                qty = open_sz
                a["unitPrice"] = 0.0
            if qty > 0:
                a["quantity"] = -qty if f["side"] == "SELL" else qty
                f["qty"] = qty
        if is_option_symbol(a.get("symbol")) and (
            "ASSIGN" in compact(a.get("rawType")) or "ASSIGN" in compact(a.get("activityType"))
        ):
            a["unitPrice"] = 0.0
        if f["qty"] > 0:
            closing_dir = "SHORT" if f["side"] == "BUY" else "LONG"
            opening = opening_direction(a, f["side"])
            left = f["qty"]
            close_amt = min(left, rem.get(closing_dir, 0.0))
            rem[closing_dir] -= close_amt
            left -= close_amt
            if left > EPS and is_option_symbol(a.get("symbol")):
                pool = pool_of(a)
                pooled = min(left, pool[closing_dir])
                pool[closing_dir] -= pooled
                left -= pooled
            if left > EPS and opening:
                rem[opening] += left


# --------------------------------------------------------------------------
# FIFO matching
# --------------------------------------------------------------------------


def stable_trade_id(t):
    return "|".join(
        [
            _s(t.get("accountId")),
            _s(t.get("symbol")),
            _s(t.get("currency")),
            _s(t.get("entryDate")),
            _s(t.get("exitDate")),
            fmt8(t.get("quantity")),
            fmt8(t.get("entryPrice")),
            fmt8(t.get("exitPrice")),
            _s(t.get("side")),
        ]
    )


def slice_member_key(t):
    buy = _s(t.get("buyActivityId"))
    sell = _s(t.get("sellActivityId"))
    if buy and sell:
        return "|".join([buy, sell, fmt8(t.get("quantity"))])
    return _s(t.get("sliceKey") or t.get("id"))


def group_id_for_keys(keys):
    """Legacy ledger.html group id: FNV-1a over the sorted member keys."""
    s = "\n".join(sorted(keys))
    h = 2166136261
    for ch in s:
        h ^= ord(ch)
        h = (h * 16777619) & 0xFFFFFFFF
    return "g_%s_%d" % (format(h, "x"), len(keys))


def _fill_rank(f):
    a = f["a"]
    t = compact(a.get("activityType"))
    s = compact(a.get("activitySubType"))
    blob = t + s
    if ("TOOPEN" in blob or t == "STO" or s == "STO") and f["side"] == "SELL":
        return 0
    if is_close_only(a) and f["side"] == "BUY":
        return 1
    if f["side"] == "BUY":
        return 2
    if "TOCLOSE" in blob or t == "STC" or s == "STC":
        return 3
    return 4


def _fill_sort_key(f):
    a = f["a"]
    return (_s(a.get("transactionDate")), _fill_rank(f), _s(a.get("occurredAt")), _s(a.get("id")))


def _make_slice(lot, fill, a, matched, symbol=None):
    fill_qty = fill["qty"]
    exit_commission = _num(a.get("commission")) * (matched / fill_qty) if fill_qty > 0 else 0.0
    entry_commission = lot["commission"] * (matched / lot["qty"]) if lot["qty"] > 0 else 0.0
    commission = entry_commission + exit_commission
    sym = symbol or lot["symbol"]
    mult = option_multiplier(sym)
    exit_px = _num(a.get("unitPrice"))
    if lot["direction"] == "LONG":
        raw_pnl = (exit_px - lot["price"]) * matched * mult
    else:
        raw_pnl = (lot["price"] - exit_px) * matched * mult
    t = {
        "id": "",
        "rt": lot["rt"],
        "accountId": lot["accountId"],
        "accountType": lot["accountType"],
        "account": lot["accountType"],
        "symbol": sym,
        "name": a.get("name") or lot["name"] if symbol else lot["name"],
        "currency": lot["currency"],
        "kind": lot["kind"],
        "side": fill["side"],
        "quantity": matched,
        "entryPrice": lot["price"],
        "exitPrice": exit_px,
        "entryDate": lot["date"],
        "exitDate": _s(a.get("transactionDate")),
        "entryWhen": lot["when"],
        "exitWhen": _s(a.get("occurredAt")),
        "holdDays": days_between(lot["date"], a.get("transactionDate")),
        "commission": commission,
        "entryCommission": entry_commission,
        "exitCommission": exit_commission,
        "pnl": raw_pnl - commission,
        "pnlCad": raw_pnl - commission,
        "openDirection": lot["direction"],
        "buyActivityId": lot["activityId"],
        "sellActivityId": _s(a.get("id")),
        "securityId": lot.get("securityId") or a.get("securityId") or "",
        "flags": sorted(set(lot.get("flags", [])) | set(a.get("flags", []))),
    }
    t["id"] = stable_trade_id(t)
    return t


def _dust(remaining, fill, a):
    """Sells that exceed the lots by a residue are rounding, not a short.

    Crypto quantities come back from Wealthsimple net of in-kind fees and
    rewards, so up to 1% of the fill is tolerated there; for everything else
    only float noise or less than a cent of value is ignored."""
    qty = fill["qty"] or 0.0
    px = abs(_num(a.get("unitPrice")))
    if remaining <= 1e-6 * max(1.0, qty):
        return True
    if (a.get("kind") or kind_of(a)) == "Crypto" and remaining <= 0.01 * qty:
        return True
    return px > 0 and remaining * px * option_multiplier(a.get("symbol")) < 0.01


def match_fifo(activities):
    """FIFO per (account, symbol, currency). Returns closed slices, open lots, unmatched."""
    activities = [a if "flags" in a else normalize_activity(a) for a in activities or []]
    normalized = [a for a in activities if "pending-distribution" not in (a.get("flags") or [])]
    folded = fold_stkdis(normalized)
    fills = []
    for a in folded:
        if a.get("category") not in ("trade", "option_event") or not a.get("symbol"):
            continue
        side = store.trade_side(a)
        if not side:
            continue
        fills.append({"a": a, "side": side, "qty": abs(_num(a.get("quantity")))})
    fills.sort(key=_fill_sort_key)
    infer_zero_qty_option_fills(fills)
    usable = [f for f in fills if f["qty"] > 0]

    books = {}
    rt_open = {}
    closed = []
    unmatched = []
    rolled = {}  # (account, underlying, right) -> {"LONG": [lots], "SHORT": [lots]}: legs Wealthsimple never posted

    def rolled_of(a):
        k = roll_key(a)
        if k not in rolled:
            rolled[k] = {"LONG": [], "SHORT": []}
        return rolled[k]

    rolled_keys = set()

    def close_rolled(fill, a, remaining, closing_dir):
        """Close carried-forward legs against this fill; they take this contract's
        symbol. When this chain has been rolled, a buy-back beyond the known
        shorts also closes the chain's older contracts (nearest expiry first):
        those are the legs the rolls moved here without posting."""
        pool = rolled_of(a)
        lots = pool[closing_dir]
        key = book_key(a)
        # A rolled chain is one position from the first short to the last
        # buy-back: everything this fill closes shares one round trip, the
        # contract's own if it has lots, otherwise the chain's.
        rt = fill.get("rtBefore") or pool.get("rt") or (lots and (lots[0].get("rt") or "rt:" + _s(lots[0].get("activityId")))) or None
        if rt:
            pool["rt"] = rt
        while remaining > EPS and lots:
            lot = lots[0]
            lot["symbol"] = _s(a.get("symbol"))
            lot["rt"] = rt or lot.get("rt") or ("rt:" + _s(lot.get("activityId")))
            matched = min(lot["qty"], remaining)
            closed.append(_make_slice(lot, fill, a, matched))
            lot["qty"] -= matched
            remaining -= matched
            if lot["qty"] <= EPS:
                lots.pop(0)
        if remaining > EPS and roll_key(a) in rolled_keys:
            others = []
            for k2, b2 in books.items():
                if k2 == key or not b2:
                    continue
                bits = k2.split("::")
                if bits[0] != fifo_account(a) or bits[2] != _s(a.get("currency")):
                    continue
                if not is_option_symbol(bits[1]) or underlying_symbol(bits[1]) != underlying_symbol(a.get("symbol")) or option_right(bits[1]) != option_right(a.get("symbol")):
                    continue
                others.append((option_expiry(bits[1]), k2))
            for _, k2 in sorted(others):
                b2 = books[k2]
                while remaining > EPS and b2 and b2[0]["direction"] == closing_dir:
                    lot = b2[0]
                    matched = min(lot["qty"], remaining)
                    s = _make_slice(lot, fill, a, matched, _s(a.get("symbol")))
                    s["flags"] = sorted(set(s["flags"]) | {"rolled-in"})
                    if rt:
                        s["rt"] = rt
                    closed.append(s)
                    lot["qty"] -= matched
                    remaining -= matched
                    if lot["qty"] <= EPS:
                        b2.pop(0)
                if not b2:
                    rt_open[k2] = None
        if not pool["LONG"] and not pool["SHORT"] and not books.get(key):
            pool["rt"] = None
        return remaining

    replaced = replacement_index(normalized)
    splits = split_markers(normalized)
    pending_splits = {}
    for (acct, sym, day), factor in splits.items():
        pending_splits.setdefault(acct + "::" + sym, []).append((day, factor))

    def apply_splits(key, day):
        skey = "::".join(key.split("::")[:2])
        todo = pending_splits.get(skey)
        if not todo:
            return
        keep = []
        for split_day, factor in sorted(todo):
            if split_day <= day:
                for lot in books.get(key, []):
                    lot["qty"] *= factor
                    lot["price"] /= factor
                    lot.setdefault("flags", [])
                    label = "split %s" % ("1:%d" % round(1 / factor) if factor < 1 else "%d:1" % round(factor))
                    if label not in lot["flags"]:
                        lot["flags"].append(label)
            else:
                keep.append((split_day, factor))
        if keep:
            pending_splits[skey] = keep
        else:
            pending_splits.pop(skey, None)

    def close_against(book, key, fill, a, remaining, symbol_override=None):
        closing_dir = "SHORT" if fill["side"] == "BUY" else "LONG"
        while remaining > EPS and book and book[0]["direction"] == closing_dir:
            lot = book[0]
            matched = min(lot["qty"], remaining)
            closed.append(_make_slice(lot, fill, a, matched, symbol_override))
            lot["commission"] *= (lot["qty"] - matched) / lot["qty"] if lot["qty"] > 0 else 0.0
            lot["qty"] -= matched
            remaining -= matched
            if lot["qty"] <= EPS:
                book.pop(0)
        if not book:
            rt_open[key] = None
        return remaining

    for fill in usable:
        a = fill["a"]
        key = book_key(a)
        book = books.setdefault(key, [])
        apply_splits(key, _s(a.get("transactionDate")))
        if is_option_symbol(a.get("symbol")) and is_multileg(a) and fill.get("rollDirection"):
            # Roll: close this contract (book, then carried-forward legs) and carry
            # the same quantity to the unposted new leg. A debit belongs to the
            # closed leg's exit, a credit to the new leg's entry.
            direction = fill["rollDirection"]
            cash = _num(a.get("netCashAmount"))
            per = abs(cash) / (fill["qty"] * 100.0) if fill["qty"] > 0 else 0.0
            debit = cash < 0
            exit_px = per if (direction == "SHORT") == debit else 0.0
            entry_px = per if (direction == "SHORT") != debit else 0.0
            a["unitPrice"] = exit_px
            before = len(closed)
            fill["rtBefore"] = rt_open.get(key) if book else None
            remaining = close_against(book, key, fill, a, fill["qty"])
            remaining = close_rolled(fill, a, remaining, direction)
            moved = fill["qty"] - remaining
            rolled_keys.add(roll_key(a))
            if moved > EPS:
                for s in closed[before:]:
                    if "rolled" not in s["flags"]:
                        s["flags"].append("rolled")
                chain_rt = fill.get("rtBefore") or rolled_of(a).get("rt") or (closed[before]["rt"] if len(closed) > before else None)
                rolled_of(a)["rt"] = chain_rt
                rolled_of(a)[direction].append(
                    {
                        "qty": moved,
                        "price": entry_px,
                        "date": _s(a.get("transactionDate")),
                        "when": _s(a.get("occurredAt")),
                        "commission": 0.0,
                        "direction": direction,
                        "accountId": _s(a.get("accountId")),
                        "accountType": fifo_account(a),
                        "symbol": _s(a.get("symbol")),
                        "name": _s(a.get("name")),
                        "currency": _s(a.get("currency")),
                        "kind": "Options",
                        "activityId": _s(a.get("id")),
                        "securityId": "",
                        "rt": chain_rt,
                        "flags": ["rolled-in"],
                    }
                )
            if remaining > EPS:
                # nothing to roll: this multileg simply opened a position
                opening = "LONG" if debit else "SHORT"
                a["unitPrice"] = per
                fill["side"] = "BUY" if opening == "LONG" else "SELL"
                if not book or not rt_open.get(key):
                    rt_open[key] = "rt:" + _s(a.get("id"))
                book.append({"qty": remaining, "price": per, "date": _s(a.get("transactionDate")), "when": _s(a.get("occurredAt")), "commission": 0.0, "direction": opening, "accountId": _s(a.get("accountId")), "accountType": fifo_account(a), "symbol": _s(a.get("symbol")), "name": _s(a.get("name")), "currency": _s(a.get("currency")), "kind": "Options", "activityId": _s(a.get("id")), "securityId": _s(a.get("securityId")), "rt": rt_open[key], "flags": list(a.get("flags") or [])})
            continue
        if "transfer-out" in (a.get("flags") or []):
            # coins sent out of the account leave at cost: off the open lots
            # first-in first-out, no slice, no P&L, not a fill of the trade
            remaining = fill["qty"]
            while remaining > EPS and book and book[0]["direction"] == "LONG":
                lot = book[0]
                matched = min(lot["qty"], remaining)
                lot["commission"] *= (lot["qty"] - matched) / lot["qty"] if lot["qty"] > 0 else 0.0
                lot["qty"] -= matched
                remaining -= matched
                if lot["qty"] <= EPS:
                    book.pop(0)
            if not book:
                rt_open[key] = None
            continue
        fill["rtBefore"] = rt_open.get(key) if book else None
        remaining = close_against(book, key, fill, a, fill["qty"])
        if remaining > EPS and is_option_symbol(a.get("symbol")):
            remaining = close_rolled(fill, a, remaining, "SHORT" if fill["side"] == "BUY" else "LONG")
        if remaining > EPS and fill["side"] == "SELL":
            for dk, dbook in books.items():
                if not dbook or dk == key:
                    continue
                bits = dk.split("::")
                if bits[0] != fifo_account(a) or bits[2] != _s(a.get("currency")):
                    continue
                if not ticker_was_replaced(replaced, bits[0], bits[1], bits[2], _s(a.get("transactionDate"))):
                    continue
                remaining = close_against(dbook, dk, fill, a, remaining, symbol_override=a.get("symbol"))
                if remaining <= EPS:
                    break
        if remaining > EPS and fill["side"] == "SELL" and not opening_direction(a, fill["side"]) and _dust(remaining, fill, a):
            remaining = 0.0
        if remaining > EPS:
            opening = opening_direction(a, fill["side"])
            if opening:
                if not book or not rt_open.get(key):
                    rt_open[key] = "rt:" + _s(a.get("id"))
                book.append(
                    {
                        "qty": remaining,
                        "price": _num(a.get("unitPrice")),
                        "date": _s(a.get("transactionDate")),
                        "when": _s(a.get("occurredAt")),
                        "commission": _num(a.get("commission")) * (remaining / fill["qty"]) if fill["qty"] > 0 else 0.0,
                        "direction": opening,
                        "accountId": _s(a.get("accountId")),
                        "accountType": fifo_account(a),
                        "symbol": _s(a.get("symbol")),
                        "name": _s(a.get("name")),
                        "currency": _s(a.get("currency")),
                        "kind": a.get("kind") or kind_of(a),
                        "activityId": _s(a.get("id")),
                        "securityId": _s(a.get("securityId")),
                        "rt": rt_open[key],
                        "flags": list(a.get("flags") or []),
                    }
                )
            else:
                unmatched.append(
                    {
                        "symbol": a.get("symbol"),
                        "currency": a.get("currency"),
                        "side": fill["side"],
                        "quantity": remaining,
                        "price": _num(a.get("unitPrice")),
                        "date": a.get("transactionDate"),
                        "description": a.get("description") or "",
                        "accountId": a.get("accountId"),
                        "account": fifo_account(a),
                        "activityId": a.get("id"),
                    }
                )

    for key in list(books):
        apply_splits(key, "9999-12-31")
    for (acct, under, right), dirs in rolled.items():
        for direction in ("LONG", "SHORT"):
            for lot in dirs[direction]:
                if lot["qty"] <= EPS:
                    continue
                # the closing leg of this roll was never posted; the credit (or
                # nothing, for a debit roll) is what it earned
                pseudo = {"a": {"id": "roll-out:" + lot["activityId"], "unitPrice": 0.0, "commission": 0.0, "transactionDate": lot["date"], "occurredAt": lot["when"], "name": lot["name"], "securityId": "", "flags": ["rolled-out"]}, "side": "BUY" if direction == "SHORT" else "SELL", "qty": lot["qty"]}
                lot["rt"] = lot.get("rt") or "rt:" + lot["activityId"]
                s = _make_slice(lot, pseudo, pseudo["a"], lot["qty"])
                s["sellActivityId"] = ""
                closed.append(s)
    open_lots = []
    for book in books.values():
        for lot in book:
            if lot["qty"] <= 1e-6:
                continue
            # crypto residue from in-kind fees: a lot worth under a dollar is not a position
            if lot["kind"] == "Crypto" and lot["qty"] * lot["price"] < 1.0:
                continue
            open_lots.append(dict(lot))
    closed.sort(key=lambda t: (t["exitDate"], t["id"]))
    fold_option_rolls(closed, open_lots)
    return {"closed": closed, "open": open_lots, "unmatched": unmatched}


def fold_option_rolls(closed, open_lots):
    """Same-day cover + new short on the same underlying is a roll: fold the
    cover's P&L into the far contract's basis and drop the cover row."""
    if not closed:
        return

    def roll_book(t):
        return "::".join([_s(t.get("account") or t.get("accountType")), _s(t.get("currency")), underlying_symbol(t.get("symbol"))])

    def day_of(s):
        return _s(s)[:10]

    covers = [t for t in closed if t["openDirection"] == "SHORT" and is_option_symbol(t["symbol"])]
    covers.sort(key=lambda t: (day_of(t["entryDate"]), day_of(t["exitDate"]), _s(t["id"])))
    drop = set()
    for cover in covers:
        if cover["id"] in drop:
            continue
        d = day_of(cover["exitDate"])
        if not d:
            continue
        under = underlying_symbol(cover["symbol"])
        if not under or under == "—":
            continue
        ck = roll_book(cover)
        closed_cands = [
            t
            for t in closed
            if t["id"] != cover["id"]
            and t["id"] not in drop
            and t["openDirection"] == "SHORT"
            and is_option_symbol(t["symbol"])
            and t["symbol"] != cover["symbol"]
            and roll_book(t) == ck
            and day_of(t["entryDate"]) == d
        ]
        open_cands = [
            l
            for l in open_lots
            if l["direction"] == "SHORT"
            and is_option_symbol(l["symbol"])
            and l["symbol"] != cover["symbol"]
            and "::".join([l["accountType"], l["currency"], underlying_symbol(l["symbol"])]) == ck
            and day_of(l["date"]) == d
        ]
        kind = "closed" if closed_cands else "open"
        cands = closed_cands if closed_cands else open_cands
        if not cands:
            continue
        cq = abs(_num(cover["quantity"]))
        qty_field = "quantity" if kind == "closed" else "qty"
        cands.sort(key=lambda r: (abs(abs(_num(r[qty_field])) - cq), _s(r["symbol"])))
        row = cands[0]
        qty = abs(_num(row[qty_field]))
        if not qty > 0:
            continue
        adj = _num(cover["pnl"]) / (qty * option_multiplier(row["symbol"]))
        if kind == "closed":
            row["entryPrice"] = _num(row["entryPrice"]) + adj
            mult = option_multiplier(row["symbol"])
            raw = (
                (row["entryPrice"] - row["exitPrice"]) if row["openDirection"] == "SHORT" else (row["exitPrice"] - row["entryPrice"])
            ) * qty * mult
            row["pnl"] = raw - _num(row.get("commission"))
            row["pnlCad"] = row["pnl"]
            row["id"] = stable_trade_id(row)
            row.setdefault("flags", [])
            if "rolled" not in row["flags"]:
                row["flags"].append("rolled")
        else:
            row["price"] = _num(row["price"]) + adj
            row.setdefault("flags", [])
            if "rolled" not in row["flags"]:
                row["flags"].append("rolled")
        drop.add(cover["id"])
    if drop:
        closed[:] = [t for t in closed if t["id"] not in drop]


_EXPIRY_RE = re.compile(r"^\S+ (\d{2})([A-Z]{3})(\d{2}) ")


def option_expiry(symbol):
    """'LUNR 29AUG25 11.50 CALL' -> '2025-08-29'."""
    m = _EXPIRY_RE.match(_SPACE_RE.sub(" ", _s(symbol).strip().upper()))
    if not m:
        return ""
    try:
        month = MONTHS.index(m.group(2).title()) + 1
    except ValueError:
        return ""
    return "20%s-%02d-%s" % (m.group(3), month, m.group(1))


def synthesize_assignment_shares(activities, securities):
    """An assigned short option delivers shares, but Wealthsimple posts only
    the option row (with the strike cash on it). Add the share leg: a call
    assignment sells contracts x 100 shares at the strike, a put assignment
    buys them."""
    out = []
    for a in activities:
        if a.get("category") != "option_event" or compact(a.get("activityType")) != "ASSIGN":
            continue
        symbol = _s(a.get("symbol"))
        if not is_option_symbol(symbol):
            continue
        contracts = abs(_num(a.get("quantity")))
        if contracts <= 0:
            continue
        shares = contracts * 100
        cash = _num(a.get("netCashAmount"))
        strike = abs(cash) / shares if abs(cash) > EPS else 0.0
        if strike <= 0:
            m = re.search(r" (\d+(?:\.\d+)?) (CALL|PUT)$", _SPACE_RE.sub(" ", symbol.upper()))
            strike = _num(m.group(1)) if m else 0.0
        if strike <= 0:
            continue
        is_call = symbol.upper().rstrip().endswith("CALL") or symbol.upper().rstrip().endswith(" C")
        sell = is_call if abs(cash) <= EPS else cash > 0
        under = underlying_symbol(symbol)
        sec = securities.by_id.get(_s(a.get("securityId")))
        under_id = _s((sec or {}).get("underlyingId")) or None
        out.append(
            {
                "id": "assign-shares:" + _s(a.get("id")),
                "canonicalId": None,
                "occurredAt": _s(a.get("occurredAt")) or _s(a.get("transactionDate")) + "T21:30:00+00:00",
                "transactionDate": _s(a.get("transactionDate")),
                "settlementDate": _s(a.get("transactionDate")),
                "accountId": _s(a.get("accountId")),
                "bookId": _s(a.get("bookId") or a.get("accountId")),
                "fifoId": _s(a.get("fifoId") or a.get("accountId")),
                "accountType": a.get("accountType"),
                "activityType": "Trade",
                "activitySubType": "SELL" if sell else "BUY",
                "description": ("Called away" if sell else "Put to you") + ": %s %s @ %s" % (shares, under, strike),
                "direction": "CREDIT" if sell else "DEBIT",
                "symbol": under,
                "name": under,
                "currency": _s(a.get("currency")),
                "quantity": -shares if sell else shares,
                "unitPrice": strike,
                "commission": 0.0,
                "netCashAmount": shares * strike if sell else -shares * strike,
                "category": "trade",
                "balance": None,
                "source": "derived",
                "rawType": "OPTIONS_ASSIGN_SHARES",
                "aftType": "",
                "counterSymbol": "",
                "securityId": under_id,
                "kind": "Shares",
                "flags": ["assignment"],
            }
        )
    return out


def synthesize_expiries(activities, open_lots, today):
    """Wealthsimple does not always post an expiry row. An option lot still
    open after its expiry date is closed at $0 on that date."""
    out = []
    seen = set()
    for lot in open_lots:
        exp = option_expiry(lot["symbol"])
        if not exp or exp >= today:
            continue
        key = (lot["accountType"], lot["symbol"], lot["currency"])
        if key in seen:
            continue
        seen.add(key)
        qty = sum(l["qty"] for l in open_lots if (l["accountType"], l["symbol"], l["currency"]) == key and l["direction"] == lot["direction"])
        if qty <= EPS:
            continue
        short = lot["direction"] == "SHORT"
        out.append(
            {
                "id": "expiry:%s|%s|%s" % (lot["accountType"], lot["symbol"], lot["currency"]),
                "canonicalId": None,
                "occurredAt": exp + "T21:30:00+00:00",
                "transactionDate": exp,
                "settlementDate": exp,
                "accountId": lot["accountId"],
                "bookId": lot["accountId"],
                "fifoId": lot["accountId"],
                "accountType": lot["accountType"],
                "activityType": "EXPIR",
                "activitySubType": "BUY" if short else "SELL",
                "description": "Expired (assumed): %s" % lot["symbol"],
                "direction": "",
                "symbol": lot["symbol"],
                "name": lot["name"],
                "currency": lot["currency"],
                "quantity": qty if short else -qty,
                "unitPrice": 0.0,
                "commission": 0.0,
                "netCashAmount": 0.0,
                "category": "option_event",
                "balance": None,
                "source": "derived",
                "rawType": "OPTIONS_SHORT_EXPIRY" if short else "OPTIONS_EXPIRY",
                "aftType": "",
                "counterSymbol": "",
                "securityId": lot.get("securityId") or None,
                "kind": "Options",
                "flags": ["assumed-expiry"],
            }
        )
    return out


# --------------------------------------------------------------------------
# FX
# --------------------------------------------------------------------------


def rate_on(fx, day):
    d = _s(day)[:10]
    if not d:
        return FX_FALLBACK
    for _ in range(12):
        r = fx.get(d)
        if r and r > 0:
            return r
        d = shift_date(d, -1)
    return FX_FALLBACK


def to_cad(fx, amount, currency, day):
    ccy = _s(currency or "CAD").upper()
    if ccy != "USD":
        return amount
    return amount * rate_on(fx, day)


def apply_fx(slices, fx):
    for t in slices:
        ccy = _s(t.get("currency") or "CAD").upper()
        if ccy != "USD":
            t["pnlCad"] = t["pnl"]
            t["feesCad"] = t["commission"]
            continue
        qty = t["quantity"]
        mult = option_multiplier(t["symbol"])
        entry_c = t.get("entryCommission") or 0.0
        exit_c = t.get("exitCommission") or 0.0
        entry_notional = t["entryPrice"] * qty * mult
        exit_notional = t["exitPrice"] * qty * mult
        if t["openDirection"] == "SHORT":
            pnl_cad = to_cad(fx, entry_notional - entry_c, ccy, t["entryDate"]) - to_cad(fx, exit_notional + exit_c, ccy, t["exitDate"])
        else:
            pnl_cad = to_cad(fx, exit_notional - exit_c, ccy, t["exitDate"]) - to_cad(fx, entry_notional + entry_c, ccy, t["entryDate"])
        t["pnlCad"] = pnl_cad
        t["feesCad"] = to_cad(fx, entry_c, ccy, t["entryDate"]) + to_cad(fx, exit_c, ccy, t["exitDate"])
    return slices


# --------------------------------------------------------------------------
# securities / exchange labels
# --------------------------------------------------------------------------

_EXCH_ALIAS = {
    "TSXV": "TSX-V",
    "TSX-V": "TSX-V",
    "TSX VENTURE": "TSX-V",
    "CDNX": "TSX-V",
    "VENTURE": "TSX-V",
    "TORONTO": "TSX",
    "TSX": "TSX",
    "CBOE CANADA": "Cboe Canada",
    "CBOE CA": "Cboe Canada",
    "NEO": "Cboe Canada",
}
_MIC_MAP = {
    "XTSV": "TSX-V",
    "XTSX": "TSX",
    "XNAS": "NASDAQ",
    "XNYS": "NYSE",
    "XASE": "NYSE American",
    "ARCX": "NYSE Arca",
    "XCNQ": "CSE",
    "NEOE": "Cboe Canada",
}


def exchange_label(sec):
    raw = _s((sec or {}).get("primaryExchange")).strip()
    up = raw.upper()
    if up in _EXCH_ALIAS:
        return _EXCH_ALIAS[up]
    if raw:
        return raw
    mic = _s((sec or {}).get("primaryMic")).upper()
    return _MIC_MAP.get(mic, "")


def listing_ticker(sym):
    s = _s(sym).strip()
    m = re.match(r"^(.+)\.(TO|V|CN|NE)$", s, re.I)
    return m.group(1) if m else s


def is_alpha_venue(sec):
    exch = _s((sec or {}).get("primaryExchange")).upper()
    mic = _s((sec or {}).get("primaryMic")).upper()
    return exch in ("ALPHA EXCHANGE", "ALPHA") or mic == "XATS"


class Securities:
    def __init__(self, rows):
        self.by_id = {}
        for r in rows or []:
            if r.get("id"):
                self.by_id[r["id"]] = r

    def preferred(self, sec):
        if not sec or not is_alpha_venue(sec):
            return sec
        sym = listing_ticker(sec.get("symbol"))
        ccy = _s(sec.get("currency"))
        if not sym:
            return sec
        for other in self.by_id.values():
            if other is sec or other.get("underlyingId"):
                continue
            if listing_ticker(other.get("symbol")) != sym:
                continue
            if ccy and _s(other.get("currency")) and _s(other.get("currency")) != ccy:
                continue
            if is_alpha_venue(other) or not exchange_label(other):
                continue
            return other
        return sec

    def listing(self, security_id):
        sec = self.by_id.get(_s(security_id))
        if sec and sec.get("underlyingId"):
            under = self.by_id.get(sec["underlyingId"])
            if under:
                sec = under
        return self.preferred(sec)

    def exchange(self, security_id):
        return exchange_label(self.listing(security_id))

    def name(self, security_id, fallback=""):
        sec = self.listing(security_id)
        return _s((sec or {}).get("name")) or fallback

    def cash_currencies(self):
        """Security id -> currency for the cash rows Wealthsimple lists as securities (CAD, USD)."""
        out = {}
        for sid, sec in self.by_id.items():
            sym = _s(sec.get("symbol")).upper()
            if sym in ("CAD", "USD") or _s(sid).startswith("sec-c-"):
                out[sid] = _s(sec.get("currency")).upper() or sym
        return out

    def known_exchanges(self):
        out = set()
        for sec in self.by_id.values():
            e = self.exchange(sec.get("id"))
            if e:
                out.add(e)
        return sorted(out)


# --------------------------------------------------------------------------
# trades (round trips), positions, cashflow
# --------------------------------------------------------------------------


def _slim_slice(s):
    return {
        "key": slice_member_key(s),
        "qty": s["quantity"],
        "entry": s["entryPrice"],
        "exit": s["exitPrice"],
        "entryDate": s["entryDate"],
        "exitDate": s["exitDate"],
        "pnl": s["pnl"],
        "pnlCad": s["pnlCad"],
        "fees": s["commission"],
        "buyActivityId": s["buyActivityId"],
        "sellActivityId": s["sellActivityId"],
        "flags": s.get("flags", []),
    }


def _fill_row(a):
    day, clock = when_parts(a.get("occurredAt") or a.get("transactionDate"))
    side = store.trade_side(a)
    qty = abs(_num(a.get("quantity")))
    return {
        "id": _s(a.get("id")),
        "when": _s(a.get("occurredAt") or a.get("transactionDate")),
        "date": day or _s(a.get("transactionDate")),
        "time": clock,
        "side": side,
        "sub": _s(a.get("activitySubType")),
        "qty": -qty if side == "SELL" else qty,
        "price": _num(a.get("unitPrice")),
        "amount": _num(a.get("netCashAmount")),
        "fees": _num(a.get("commission")),
        "currency": _s(a.get("currency")),
        "flags": a.get("flags", []),
    }


def collapse_trade(gid, slices, locked, status, acts_by_id, securities, journal):
    slices = sorted(slices, key=lambda s: (s["exitDate"], s["entryDate"], slice_member_key(s)))
    t0 = slices[0]
    qty = sum(s["quantity"] for s in slices)
    entry_notional = sum(s["entryPrice"] * s["quantity"] for s in slices)
    exit_notional = sum(s["exitPrice"] * s["quantity"] for s in slices)
    pnl = sum(s["pnl"] for s in slices)
    pnl_cad = sum(s["pnlCad"] for s in slices)
    fees = sum(s["commission"] for s in slices)
    fees_cad = sum(s.get("feesCad", s["commission"]) for s in slices)
    entry_date = min(s["entryDate"] for s in slices)
    exit_date = max(s["exitDate"] for s in slices)
    entry_when = min(s.get("entryWhen") or s["entryDate"] for s in slices)
    exit_when = max(s.get("exitWhen") or s["exitDate"] for s in slices)
    mult = option_multiplier(t0["symbol"])
    entry = entry_notional / qty if qty else 0.0
    exit_px = exit_notional / qty if qty else t0["exitPrice"]
    basis = abs(entry * qty * mult)
    sec_id = next((s.get("securityId") for s in slices if s.get("securityId")), "")
    ids = []
    for s in slices:
        for k in ("buyActivityId", "sellActivityId"):
            if s.get(k) and s[k] not in ids:
                ids.append(s[k])
    fills = [_fill_row(acts_by_id[i]) for i in ids if i in acts_by_id]
    # label each fill by what it did in this trade, not by the broker's order
    # type: the open/close order types are option language, shares and crypto
    # fills are simply bought or sold
    opened_ids = {s.get("buyActivityId") for s in slices}
    closed_ids = {s.get("sellActivityId") for s in slices}
    for f in fills:
        opened, closed = f["id"] in opened_ids, f["id"] in closed_ids
        side = "BUY" if f["side"] == "BUY" else "SELL"
        if t0["kind"] != "Options":
            f["sub"] = side + (" (close + open)" if opened and closed else "")
        elif closed and not opened:
            f["sub"] = side + " TO CLOSE"
        elif opened and not closed:
            f["sub"] = side + " TO OPEN"
        elif opened and closed:
            f["sub"] = side + " (close + open)"
    fills.sort(key=lambda f: f["when"], reverse=True)
    open_side = "BUY" if t0["openDirection"] == "LONG" else "SELL"
    opens = [f for f in fills if f["side"] == open_side]
    closes = [f for f in fills if f["side"] != open_side]
    flags = sorted({fl for s in slices for fl in s.get("flags", [])})
    entry_j = journal.get(gid) or {}
    return {
        "id": gid,
        "status": status,
        "locked": bool(locked),
        "symbol": t0["symbol"],
        "underlying": underlying_symbol(t0["symbol"]),
        "name": securities.name(sec_id, t0.get("name") or t0["symbol"]),
        "exchange": securities.exchange(sec_id) if t0["kind"] != "Crypto" else "Crypto",
        "kind": t0["kind"],
        "currency": t0["currency"],
        "account": t0["account"],
        "accountId": t0["accountId"],
        "securityId": sec_id,
        "side": "SELL" if t0["openDirection"] == "LONG" else "COVER",
        "openDirection": t0["openDirection"],
        "qty": qty,
        "mult": mult,
        "entry": entry,
        "exit": exit_px,
        "entryDate": entry_date,
        "exitDate": exit_date,
        "entryWhen": entry_when,
        "exitWhen": exit_when,
        "holdDays": days_between(entry_date, exit_date),
        "pnl": pnl,
        "pnlCad": pnl_cad,
        "fees": fees,
        "feesCad": fees_cad,
        "pnlPct": (pnl / basis) if basis > 0 else None,
        "legs": [_slim_slice(s) for s in slices],
        "legCount": len(slices),
        "fills": fills,
        "opened": {"qty": qty, "avg": entry, "fills": len(opens)},
        "closed": {"qty": qty, "avg": exit_px, "fills": len(closes)},
        "netCash": pnl,
        "flags": flags,
        "grade": entry_j.get("grade", ""),
        "thesis": entry_j.get("thesis", ""),
        "tags": list(entry_j.get("tags", [])),
    }


def build_trades(closed, open_lots, saved_groups, acts_by_id, securities, journal):
    by_key = {slice_member_key(s): s for s in closed}
    used = set()
    groups = []
    for rec in saved_groups or []:
        members = []
        for k in rec.get("members") or []:
            s = by_key.get(_s(k))
            mk = slice_member_key(s) if s else ""
            if s and mk not in used:
                members.append(s)
                used.add(mk)
        if members:
            groups.append((_s(rec.get("id")) or group_id_for_keys([slice_member_key(m) for m in members]), members, True))
    by_rt = {}
    order = []
    for s in closed:
        if slice_member_key(s) in used:
            continue
        rt = s.get("rt") or "rt:" + slice_member_key(s)
        if rt not in by_rt:
            by_rt[rt] = []
            order.append(rt)
        by_rt[rt].append(s)
    for rt in order:
        groups.append((rt, by_rt[rt], False))
    trades = []
    for gid, members, locked in groups:
        trades.append(collapse_trade(gid, members, locked, "closed", acts_by_id, securities, journal))
    trades.sort(key=lambda t: (t["exitDate"], t["id"]), reverse=True)
    return trades


def last_fill_prices(activities):
    """symbol -> {price, date} from the newest fill with a price."""
    out = {}
    for a in sorted(activities, key=lambda a: (_s(a.get("transactionDate")), _s(a.get("occurredAt")))):
        if a.get("category") not in ("trade", "option_event"):
            continue
        px = _num(a.get("unitPrice"))
        if px > 0 and a.get("symbol"):
            out[a["symbol"]] = {"price": px, "date": _s(a.get("transactionDate"))}
    return out


def quote_fits(quote, kind):
    """A quote prices a position only when its source is the kind's: the coin BTC's
    Coinbase price must never price a share or a warrant called BTC, and a listing's
    TMX price never a coin. A quote with no source stated is taken as the kind's own."""
    source = _s((quote or {}).get("source"))
    if not source:
        return True
    if kind == "Crypto":
        return source == "coinbase"
    if kind == "Options":
        return source == "cboe_options"
    return source not in ("coinbase", "cboe_options")


def build_positions(open_lots, last_prices, balances, accounts, securities, journal, today, quotes=None, acts_by_id=None):
    quotes = quotes or {}
    acts_by_id = acts_by_id or {}
    nick_ids = {}
    for acc in accounts or []:
        nick = norm_account_name(acc.get("nickname") or acc.get("unifiedAccountType") or acc.get("type"))
        nick_ids.setdefault(nick, set()).add(_s(acc.get("id")))
    bal = {}
    for b in balances or []:
        bal[(_s(b.get("accountId")), _s(b.get("securityId")))] = bal.get((_s(b.get("accountId")), _s(b.get("securityId"))), 0.0) + _num(b.get("quantity"))

    groups = {}
    order = []
    for lot in open_lots:
        k = (lot["symbol"], lot["accountType"], lot["currency"], lot["direction"])
        if k not in groups:
            groups[k] = []
            order.append(k)
        groups[k].append(lot)

    rows = []
    for k in order:
        lots = sorted(groups[k], key=lambda l: (l["date"], l["when"]))
        symbol, account, currency, direction = k
        mult = option_multiplier(symbol)
        qty = sum(l["qty"] for l in lots)
        if qty <= 1e-9:
            continue
        cost = sum(l["qty"] * l["price"] * mult for l in lots)
        fees = sum(l.get("commission", 0.0) for l in lots)
        sec_id = next((l.get("securityId") for l in lots if l.get("securityId")), "")
        last = last_prices.get(symbol)
        last_px = last["price"] if last else (cost / (qty * mult) if qty else 0.0)
        last_at = last["date"] if last else ""
        price_source = "fill"
        quote = quotes.get(symbol)
        if quote and not quote_fits(quote, lots[0]["kind"]):
            quote = None
        if quote and _num(quote.get("price"), None):
            last_px = _num(quote.get("price"))
            last_at = _s(quote.get("fetchedAt"))
            price_source = "quote"
        mv = qty * last_px * mult
        unreal = (mv - cost) if direction == "LONG" else (cost - mv)
        held = sum(l["qty"] * days_between(l["date"], today) for l in lots)
        ws_qty = None
        if sec_id and account in nick_ids:
            total = 0.0
            found = False
            for aid in nick_ids[account]:
                if (aid, sec_id) in bal:
                    total += bal[(aid, sec_id)]
                    found = True
            if found:
                ws_qty = total
        # A position and the trade it becomes when it closes share one journal
        # entry: both are keyed by the round trip that opened the position.
        legacy_pid = "pos:" + "|".join([account, symbol, currency])
        pid = lots[0].get("rt") or legacy_pid
        entry_j = journal.get(pid) or journal.get(legacy_pid) or {}
        price_change = _num(quote.get("priceChange"), None) if quote else None
        fills = [_fill_row(acts_by_id[l["activityId"]]) for l in lots if l.get("activityId") in acts_by_id]
        fills.sort(key=lambda f: f["when"], reverse=True)
        rows.append(
            {
                "id": pid,
                "symbol": symbol,
                "underlying": underlying_symbol(symbol),
                "name": securities.name(sec_id, lots[0].get("name") or symbol),
                "exchange": securities.exchange(sec_id) if lots[0]["kind"] != "Crypto" else "Crypto",
                "kind": lots[0]["kind"],
                "account": account,
                "accountId": lots[0]["accountId"],
                "currency": currency,
                "securityId": sec_id,
                "short": direction == "SHORT",
                "qty": qty,
                "mult": mult,
                "avg": cost / (qty * mult) if qty else 0.0,
                "cost": cost,
                "fees": fees,
                "last": last_px,
                "lastAt": last_at,
                "priceSource": price_source,
                "priceChange": price_change,
                "percentChange": _num(quote.get("percentChange"), None) if quote else None,
                # the day's move on the whole position, in its own currency, from the quote's change
                "dayChange": (qty * price_change * mult * (-1 if direction == "SHORT" else 1)) if price_change is not None else None,
                "mv": mv,
                "unreal": unreal,
                "unrealPct": (unreal / cost) if cost else None,
                "held": round(held / qty) if qty else 0,
                "opened": lots[0]["date"],
                "wsQty": ws_qty,
                "rt": lots[0].get("rt"),
                "lots": [
                    {
                        "opened": l["date"],
                        "qty": l["qty"],
                        "price": l["price"],
                        "basis": l["qty"] * l["price"] * mult,
                        "held": days_between(l["date"], today),
                        "flags": l.get("flags", []),
                        "activityId": l.get("activityId"),
                    }
                    for l in lots
                ],
                "fills": fills,
                "grade": entry_j.get("grade", ""),
                "thesis": entry_j.get("thesis", ""),
                "tags": list(entry_j.get("tags", [])),
            }
        )
    book = sum(abs(r["cost"]) for r in rows)
    for r in rows:
        r["alloc"] = (abs(r["cost"]) / book) if book else 0.0
    rows.sort(key=lambda r: r["alloc"], reverse=True)
    return rows


_CASH_KINDS = {
    "dividend": "Dividend",
    "interest": "Interest",
}


def build_cashflow(activities, securities, fx):
    rows = []
    for a in activities:
        cat = _s(a.get("category"))
        raw = compact(a.get("rawType"))
        at = compact(a.get("activityType"))
        cash = _num(a.get("netCashAmount"))
        kind = ""
        if cat == "dividend":
            kind = "Dividend"
        elif cat == "interest":
            kind = "Interest"
        elif raw == "WITHHOLDINGTAX" or at == "WITHHOLDINGTAX":
            kind = "Withholding tax"
        elif raw == "INTERESTCHARGE" or at == "INTERESTCHARGE":
            kind = "Interest charge"
        else:
            continue
        if abs(cash) < EPS:
            continue
        day, clock = when_parts(a.get("occurredAt") or a.get("transactionDate"))
        symbol = _s(a.get("symbol")).strip() or ("Cash" if kind in ("Interest", "Interest charge") else "")
        rows.append(
            {
                "id": _s(a.get("id")),
                "date": _s(a.get("transactionDate")) or day,
                "time": clock,
                "symbol": symbol or "—",
                "name": securities.name(a.get("securityId"), _s(a.get("name")) if _s(a.get("name")) != symbol else ""),
                "kind": kind,
                "account": norm_account_name(a.get("accountType")) or _s(a.get("accountId")),
                "accountId": _s(a.get("accountId")),
                "qty": _num(a.get("quantity")) or None,
                "per": _num(a.get("unitPrice")) or None,
                "amount": cash,
                "currency": _s(a.get("currency") or "CAD"),
                "amountCad": to_cad(fx, cash, a.get("currency"), a.get("transactionDate")),
            }
        )
    rows.sort(key=lambda r: (r["date"], r["id"]), reverse=True)
    return rows


# --------------------------------------------------------------------------
# NAV: equity series, yearly time-weighted returns, drawdown
# --------------------------------------------------------------------------


def equity_series(points):
    out = []
    for p in points or []:
        d = _s(p.get("date"))[:10]
        v = _num(p.get("equity"), None)
        if not d or v is None:
            continue
        out.append({"d": d, "v": v, "dep": _num(p.get("netDeposits"), None)})
    out.sort(key=lambda p: p["d"])
    return out


def _nav_on(series, day):
    v = None
    for p in series:
        if p["d"] > day:
            break
        v = p["v"]
    return v


def _deposits_on(series, day):
    v = None
    for p in series:
        if p["d"] > day:
            break
        if p["dep"] is not None:
            v = p["dep"]
    return v


def year_return(series, year, today):
    """Daily chain-linked return for one calendar year, net of deposits."""
    cal = "%s-01-01" % year
    to = min("%s-12-31" % year, today)
    if not series:
        return None
    # A balance under 1% of the account's peak is pre-history (a few dollars
    # parked before the real start): a chain that began there would turn the
    # first big deposit into a wild return, so the chain starts at the first
    # point that clears the floor, and the year is measured from there.
    floor = max(p["v"] for p in series) * 0.01
    start_day = shift_date(cal, -1)
    start = _nav_on(series, start_day)
    after = start_day
    if not (start and start > floor):
        first = next((p for p in series if cal <= p["d"] <= to and p["v"] > floor), None)
        if not first:
            return None
        start = first["v"]
        after = first["d"]
    pts = [p for p in series if after < p["d"] <= to]
    if not pts:
        return None
    prev_eq = start
    prev_dep = _deposits_on(series, after)
    factor = 1.0
    for p in pts:
        eq = p["v"]
        if not prev_eq > 0:
            return None
        cf = 0.0
        if p["dep"] is not None and prev_dep is not None:
            cf = p["dep"] - prev_dep
        factor *= 1 + (eq - prev_eq - cf) / prev_eq
        prev_eq = eq
        if p["dep"] is not None:
            prev_dep = p["dep"]
    r = factor - 1
    if r != r or r in (float("inf"), float("-inf")):
        return None
    span_from = cal if after == start_day else after
    return {"r": r, "from": span_from, "to": to, "days": days_between(span_from, to)}


def benchmark_return(bench, year, today, start=None):
    """The index over the same span the account's year covers: the calendar year,
    or from `start` when the account was funded part way through it."""
    if not bench:
        return None
    days = sorted(bench)
    cal = _s(start)[:10] or "%s-01-01" % year
    to = min("%s-12-31" % year, today)
    prev = None
    end = None
    for d in days:
        if d < cal:
            prev = bench[d]
        elif d <= to:
            end = bench[d]
    if prev is None:
        firsts = [d for d in days if cal <= d <= to]
        if not firsts:
            return None
        prev = bench[firsts[0]]
    if not prev or end is None:
        return None
    return end / prev - 1


def yearly_returns(series, bench, today):
    if not series:
        return []
    years = sorted({p["d"][:4] for p in series})
    peak = max(p["v"] for p in series)
    out = []
    for y in years:
        # a year in which the account never held more than 1% of its peak is
        # pre-history (a few hundred dollars parked before the real start)
        year_peak = max((p["v"] for p in series if p["d"][:4] == y), default=0.0)
        if peak > 0 and year_peak < peak * 0.01:
            continue
        yr = year_return(series, y, today)
        if not yr:
            continue
        start_dep = _deposits_on(series, shift_date("%s-01-01" % y, -1))
        end_dep = _deposits_on(series, yr["to"])
        flow = (end_dep - start_dep) if (start_dep is not None and end_dep is not None) else None
        end_v = _nav_on(series, yr["to"])
        out.append(
            {
                "year": y,
                "r": yr["r"],
                "days": yr["days"],
                "from": yr["from"],
                "to": yr["to"],
                "flow": flow,
                "endV": end_v,
                "spR": benchmark_return(bench, y, today, yr["from"] if yr["from"] != "%s-01-01" % y else None),
            }
        )
    return out


def annualized(years):
    prod = 1.0
    days = 0
    used = []
    for y in years:
        if y["r"] is None or y["r"] <= -1 or y["days"] < 30:
            continue
        prod *= 1 + y["r"]
        days += y["days"]
        used.append(y["year"])
    if not days:
        return {"rate": None, "years": 0.0, "count": 0, "first": "", "last": ""}
    yrs = days / 365.25
    rate = (prod ** (1 / yrs) - 1) if yrs >= 1 / 12 else prod - 1
    return {"rate": rate, "years": yrs, "count": len(used), "first": used[0], "last": used[-1]}


def _paired_flows(series):
    """Net deposit change per day, moved one day later when the equity
    series only reflects the money a day after the deposit record does."""
    n = len(series)
    flows = [0.0] * n
    for i in range(1, n):
        p, prev = series[i], series[i - 1]
        if p["dep"] is None or prev["dep"] is None:
            continue
        cf = p["dep"] - prev["dep"]
        if abs(cf) < EPS:
            continue
        change_today = p["v"] - prev["v"]
        if i + 1 < n:
            change_next = series[i + 1]["v"] - p["v"]
            if abs(change_today - cf) > abs(change_next - cf) and abs(change_today) < abs(cf) * 0.5:
                flows[i + 1] += cf
                continue
        flows[i] += cf
    return flows


def drawdown(series):
    """Max drawdown of the flow-adjusted equity: daily returns are taken net
    of deposits and withdrawals and chain-linked into an index, so money
    moved in or out of the account is not counted as a gain or a loss."""
    if not series:
        return {"pct": None, "abs": None, "at": "", "peakAt": ""}
    peak_v = max(p["v"] for p in series)
    floor = peak_v * 0.01
    idx = 1.0
    prev = None
    peak_idx = 0.0
    peak_at = ""
    peak_equity = 0.0
    dd = 0.0
    dd_abs = 0.0
    dd_at = ""
    dd_peak_at = ""
    flows = _paired_flows(series)
    for i, p in enumerate(series):
        if prev is not None and prev["v"] > floor and prev["v"] > 0:
            idx *= 1 + (p["v"] - prev["v"] - flows[i]) / prev["v"]
        prev = p
        if p["v"] < floor:
            continue
        if idx >= peak_idx:
            peak_idx = idx
            peak_at = p["d"]
            peak_equity = p["v"]
        if peak_idx <= 0:
            continue
        drop = idx / peak_idx - 1
        if drop < dd:
            dd = drop
            dd_abs = drop * peak_equity
            dd_at = p["d"]
            dd_peak_at = peak_at
    return {"pct": dd, "abs": dd_abs, "at": dd_at, "peakAt": dd_peak_at}


# --------------------------------------------------------------------------
# base model (cached per DB version)
# --------------------------------------------------------------------------


def migrate_legacy_notes(closed, saved_groups, notes):
    """Map ledger.html note keys (hash of the slices in a lane group) onto
    round-trip ids so an existing journal is not lost."""
    if not notes:
        return {}
    by_key = {slice_member_key(s): s for s in closed}
    used = set()
    out = {}
    for rec in saved_groups or []:
        members = [by_key[_s(k)] for k in rec.get("members") or [] if _s(k) in by_key]
        if not members:
            continue
        for m in members:
            used.add(slice_member_key(m))
        gid = _s(rec.get("id"))
        if gid in notes:
            out[gid] = notes[gid]
    lanes = {}
    for s in closed:
        if slice_member_key(s) in used:
            continue
        lanes.setdefault((s["accountId"], s["symbol"], s["currency"]), []).append(s)
    for members in lanes.values():
        members.sort(key=lambda s: (s["exitDate"], s["entryDate"], slice_member_key(s)))
        cur = []
        direction = None

        def flush(cur):
            if not cur:
                return
            gid = group_id_for_keys([slice_member_key(s) for s in cur])
            if gid in notes:
                rts = {}
                for s in cur:
                    rts[s.get("rt")] = rts.get(s.get("rt"), 0) + 1
                best = max(rts, key=rts.get)
                if best:
                    out[best] = notes[gid]

        for s in members:
            if direction is not None and s["openDirection"] != direction:
                flush(cur)
                cur = []
            direction = s["openDirection"]
            cur.append(s)
        flush(cur)
    journal = {}
    for k, v in out.items():
        tags = [t.strip() for t in _s(v.get("tag")).split(",") if t.strip()]
        journal[k] = {"thesis": _s(v.get("thesis")), "tags": tags, "grade": _s(v.get("grade"))}
    return journal


def build_book(snapshot, today):
    """The matched book: every activity normalized (delivered shares and expiries
    added), the FIFO match, the securities. It depends on the activity rows, the
    securities and the day only, so a quote tick can reuse it."""
    raw_acts = snapshot.get("activities") or []
    acts = normalize_activities(raw_acts)
    securities = Securities(snapshot.get("securities") or [])
    delivered = synthesize_assignment_shares(acts, securities)
    if delivered:
        acts = acts + delivered
    fifo = match_fifo(acts)
    synthetic = synthesize_expiries(acts, fifo["open"], today)
    if synthetic:
        acts = acts + synthetic
        fifo = match_fifo(acts)
    return {"activities": acts, "actsById": {_s(a.get("id")): a for a in acts}, "securities": securities, "fifo": fifo, "rawCount": len(raw_acts)}


def build_base(snapshot, market, journal, today=None, book=None):
    today = today or today_local()
    fx = market.get("fx") or {}
    bench = market.get("benchmark") or {}
    benchmarks = dict(market.get("benchmarks") or {})
    benchmarks.setdefault("SP500", bench)
    book = book or build_book(snapshot, today)
    acts = book["activities"]
    acts_by_id = book["actsById"]
    securities = book["securities"]
    fifo = book["fifo"]
    apply_fx(fifo["closed"], fx)
    saved = snapshot.get("tradeGroups") or []
    trades = build_trades(fifo["closed"], fifo["open"], saved, acts_by_id, securities, journal)
    last_prices = last_fill_prices(acts)
    positions = build_positions(fifo["open"], last_prices, snapshot.get("balances"), snapshot.get("accounts"), securities, journal, today, market.get("quotes") or {}, acts_by_id)
    cashflow = build_cashflow(acts, securities, fx)
    equity = equity_series(snapshot.get("navHistory"))
    by_account = {}
    for nick, pts in (snapshot.get("navByAccount") or {}).items():
        by_account[norm_account_name(nick)] = equity_series(pts)
    accounts = []
    seen = set()
    for acc in snapshot.get("accounts") or []:
        nick = norm_account_name(acc.get("nickname") or acc.get("unifiedAccountType") or acc.get("type"))
        accounts.append(
            {
                "id": _s(acc.get("id")),
                "name": nick,
                "type": _s(acc.get("unifiedAccountType")),
                "currency": _s(acc.get("currency")),
                "status": _s(acc.get("status")),
                "nav": _num(acc.get("netLiquidationValue"), None),
            }
        )
        seen.add(nick)
    return {
        "today": today,
        "syncedAt": _s(snapshot.get("syncedAt")),
        "fx": fx,
        "benchmark": bench,
        "benchmarks": benchmarks,
        "distributions": market.get("distributions") or {},
        "quotes": market.get("quotes") or {},
        "fxLast": max(fx) if fx else "",
        "benchmarkLast": max(bench) if bench else "",
        "activities": acts,
        "actsById": acts_by_id,
        "securities": securities,
        "closed": fifo["closed"],
        "openLots": fifo["open"],
        "unmatched": fifo["unmatched"],
        "trades": trades,
        "positions": positions,
        "cashflow": cashflow,
        "equity": equity,
        "equityByAccount": by_account,
        "accounts": accounts,
        "balances": [dict(b) for b in (snapshot.get("balances") or []) if isinstance(b, dict)],
        "margin": [dict(m) for m in (snapshot.get("margin") or []) if isinstance(m, dict)],
        "exposures": dict(snapshot.get("exposures") or {}),
        "watchlist": [dict(w) for w in (snapshot.get("watchlist") or []) if isinstance(w, dict)],
        "tiles": snapshot.get("tiles"),
        "news": [dict(n) for n in (snapshot.get("news") or []) if isinstance(n, dict)],
        "universes": {k: [dict(r) for r in v] for k, v in (snapshot.get("universes") or {}).items()},
        "cashCurrencies": securities.cash_currencies(),
        "activityCount": book["rawCount"],
        "lastPrices": last_prices,
    }


# --------------------------------------------------------------------------
# filters and the view
# --------------------------------------------------------------------------

EMPTY_FILTERS = {
    "lists": {"account": [], "symbol": [], "grade": [], "tag": [], "kind": [], "exchange": [], "side": [], "result": []},
    "ranges": {"price": {"op": ">", "v": None}, "hold": {"op": ">", "v": None}, "pnl": {"op": ">", "v": None}, "qty": {"op": ">", "v": None}},
    "preset": "all",
    "years": [],
    "from": "",
    "to": "",
    "search": "",
    "benchmark": "SP500",
}

BENCHMARK_LABELS = {"SP500": "S&P 500", "TSX": "S&P/TSX", "TSX60": "TSX 60"}

PRESET_DAYS = {"1d": 1, "1w": 7, "1m": 30, "3m": 90, "6m": 180, "1y": 365, "5y": 1826}


def clean_filters(raw):
    f = json.loads(json.dumps(EMPTY_FILTERS))
    if not isinstance(raw, dict):
        return f
    lists = raw.get("lists") if isinstance(raw.get("lists"), dict) else {}
    for k in f["lists"]:
        vals = lists.get(k)
        if isinstance(vals, list):
            f["lists"][k] = [_s(v) for v in vals if _s(v)]
    ranges = raw.get("ranges") if isinstance(raw.get("ranges"), dict) else {}
    for k in f["ranges"]:
        r = ranges.get(k)
        if isinstance(r, dict):
            op = _s(r.get("op"))
            f["ranges"][k]["op"] = op if op in (">", "<") else ">"
            v = r.get("v")
            f["ranges"][k]["v"] = _num(v, None) if v not in (None, "") else None
    preset = _s(raw.get("preset")).lower()
    f["preset"] = preset if preset in PRESET_DAYS or preset in ("ytd", "all") else "all"
    years = raw.get("years")
    if isinstance(years, list):
        f["years"] = sorted({_s(y)[:4] for y in years if re.match(r"^\d{4}$", _s(y)[:4])})
    for k in ("from", "to"):
        v = _s(raw.get(k))[:10]
        f[k] = v if re.match(r"^\d{4}-\d{2}-\d{2}$", v) else ""
    f["search"] = _s(raw.get("search")).strip()
    b = _s(raw.get("benchmark")).strip().upper()
    f["benchmark"] = b if b in BENCHMARK_LABELS else "SP500"
    return f


def date_bounds(f, today):
    if f["from"] or f["to"]:
        return (f["from"] or "0000-01-01", f["to"] or "9999-12-31")
    if f["years"]:
        return None
    if f["preset"] == "ytd":
        return (today[:4] + "-01-01", today)
    days = PRESET_DAYS.get(f["preset"])
    if days:
        return (shift_date(today, -days), today)
    return None


def in_date_scope(f, today, day):
    b = date_bounds(f, today)
    if b:
        return b[0] <= day <= b[1]
    if f["years"]:
        return day[:4] in f["years"]
    return True


def trade_matches(t, f, today):
    s = f["search"].upper()
    if s and s not in t["symbol"].upper() and s not in t["underlying"].upper() and s not in _s(t.get("name")).upper():
        return False
    L = f["lists"]
    if L["account"] and t["account"] not in L["account"]:
        return False
    if L["symbol"] and t["symbol"] not in L["symbol"] and t["underlying"] not in L["symbol"]:
        return False
    if L["grade"] and (t["grade"] or "Ungraded") not in L["grade"]:
        return False
    if L["tag"]:
        tags = t["tags"] or ["untagged"]
        if not any(x in L["tag"] for x in tags):
            return False
    if L["kind"] and t["kind"] not in L["kind"]:
        return False
    if L["exchange"] and t["exchange"] not in L["exchange"]:
        return False
    if L["side"] and t["side"] not in L["side"]:
        return False
    if L["result"]:
        res = "Winners" if t["pnlCad"] > 0 else ("Losers" if t["pnlCad"] < 0 else "Breakeven")
        if res not in L["result"]:
            return False
    R = f["ranges"]
    for key, val in (("price", t["entry"]), ("hold", t["holdDays"]), ("pnl", t["pnlCad"]), ("qty", t["qty"])):
        r = R[key]
        if r["v"] is None:
            continue
        if r["op"] == ">" and not val > r["v"]:
            return False
        if r["op"] == "<" and not val < r["v"]:
            return False
    return in_date_scope(f, today, t["exitDate"])


UNCLASSIFIED = "Not classified"


def exposure_slices(positions, exposures, cad):
    """The open long positions in scope by sector and by country: each position's
    market value in CAD spread by its exposure record (a share's one sector and
    country; a fund's look-through), what no record covers under Not classified.
    Two lists of {name, value, share}, largest first, Not classified last."""
    sec_tot, cty_tot = {}, {}
    sec_unc = cty_unc = 0.0
    total = 0.0
    for p in positions:
        # the same positions and values as Allocation: every one worth something
        v = cad(p["mv"], p["currency"])
        if v <= 0:
            continue
        total += v
        rec = exposures.get(_s(p.get("securityId"))) or {}
        if p.get("kind") == "Options":
            # a contract is its underlying's exposure, under the share's record
            under = _s(p.get("underlying") or "").upper()
            us, ca = "share:" + under + "::US", "share:" + under + ":"   # exposure.share_exposure's keys: ticker, then the venue form
            first, second = (us, ca) if _s(p.get("currency")).upper() == "USD" else (ca, us)
            rec = exposures.get(first) or exposures.get(second) or {}
        s_map, c_map = rec.get("sectors") or {}, rec.get("countries") or {}
        if p.get("kind") == "Crypto":
            # a coin is its own sector and no country's
            s_map, c_map = {"Digital assets": 1.0}, {}
        s_sum = sum(_num(w, 0.0) for w in s_map.values())
        c_sum = sum(_num(w, 0.0) for w in c_map.values())
        for n, w in s_map.items():
            n = exposure.norm_sector(n) or n   # a record read before an alias was known folds here
            sec_tot[n] = sec_tot.get(n, 0.0) + v * _num(w, 0.0)
        for n, w in c_map.items():
            cty_tot[n] = cty_tot.get(n, 0.0) + v * _num(w, 0.0)
        sec_unc += v * max(0.0, 1.0 - min(1.0, s_sum))
        cty_unc += v * max(0.0, 1.0 - min(1.0, c_sum))

    def rows(tot, unc):
        out = [{"name": n, "value": v} for n, v in tot.items() if v > 0]
        out.sort(key=lambda x: -x["value"])
        if unc > 0.005:
            out.append({"name": UNCLASSIFIED, "value": unc})
        for x in out:
            x["share"] = (x["value"] / total) if total else 0.0
        return out
    return rows(sec_tot, sec_unc), rows(cty_tot, cty_unc)


# ---------------------------------------------------------------------------
# Markets: the watchlist with its quotes, and the heatmap's tiles
# ---------------------------------------------------------------------------
def watch_quote_key(symbol, exchange):
    """Where a watched listing's quote is kept: its symbol and venue, so a listing the
    book also holds on another venue keeps its own quote."""
    return _s(symbol).strip().upper() + "@" + _s(exchange).strip().upper()


# the Markets tab's tile row when the user has never changed it
DEFAULT_TILES = [("SPX", "INDEX"), ("NDX", "INDEX"), ("DJI", "INDEX"), ("VIX", "INDEX"), ("GC", "COMEX"), ("BTCUSD", "FX")]
TILES_MAX = 12


def tile_list(base):
    """The tile row's instruments in order: the saved set, else the default; only what the directory knows, twelve at most."""
    saved = base.get("tiles")
    rows = [(r["symbol"], r["exchange"]) for r in saved] if saved is not None else list(DEFAULT_TILES)
    out, seen = [], set()
    for sym, ex in rows:
        inst = instruments.find(sym, ex)
        if inst and inst["symbol"] not in seen:
            seen.add(inst["symbol"])
            out.append(inst)
    return out[:TILES_MAX]


def tile_symbols(base):
    """The tile row's instruments, with what the quote source needs, keyed as a watched instrument is."""
    return [{"symbol": i["symbol"], "exchange": i["exchange"], "currency": i["currency"], "kind": "Instrument",
             "quoteKey": watch_quote_key(i["symbol"], i["exchange"]), "yahoo": i["yahoo"]} for i in tile_list(base)]


def tile_decimals(inst):
    """The instrument's own price scale: two for an index or a commodity, three for a rate, four for a pair, none for Bitcoin."""
    if inst["symbol"] == "BTCUSD":
        return 0
    return {"Rate": 3, "Currency": 4}.get(inst["kind"], 2)


def tile_rows(base):
    """The Markets tab's tiles: label, last, the day's change in points and percent, and the decimals to show them with."""
    quotes = base.get("quotes") or {}
    out = []
    for inst in tile_list(base):
        q = quotes.get(watch_quote_key(inst["symbol"], inst["exchange"])) or {}
        out.append({"symbol": inst["symbol"], "exchange": inst["exchange"], "label": instruments.label(inst["symbol"]), "name": inst["name"], "kind": inst["kind"],
                    "last": _num(q.get("price"), None), "change": _num(q.get("priceChange"), None), "percentChange": _num(q.get("percentChange"), None),
                    "decimals": tile_decimals(inst)})
    return out


def watch_symbols(base=None):
    """Every watched listing, with what a quote source needs to price it."""
    base = base or base_model()
    out = []
    for w in base.get("watchlist") or []:
        inst = instruments.find(w["symbol"], w.get("exchange"))
        crypto = _s(w.get("exchange")).upper() == "CRYPTO"   # a watched coin is the USD pair, whatever currency the book holds it in
        rec = {"symbol": w["symbol"], "exchange": w.get("exchange") or "", "currency": "USD" if crypto else (w.get("currency") or ""), "kind": "Instrument" if inst else "Crypto" if crypto else "Shares",
               "quoteKey": watch_quote_key(w["symbol"], w.get("exchange"))}
        if inst:
            rec["yahoo"] = inst["yahoo"]
        out.append(rec)
    return out


def quote_symbols(base=None):
    """Everything quoted beside the book: the watched listings, then the tile row's instruments not already among them."""
    base = base or base_model()
    out = watch_symbols(base)
    keys = {r["quoteKey"] for r in out}
    for rec in tile_symbols(base):
        if rec["quoteKey"] not in keys:
            keys.add(rec["quoteKey"])
            out.append(rec)
    return out


def watch_exposure_key(symbol, exchange, currency):
    return exposure.SHARE_KEY + market.tmx_symbol(symbol) + ":" + (market.tmx_form(exchange, currency) or "")


def dominant_sector(rec):
    """The sector a record gives most weight to, or Not classified."""
    sectors = (rec or {}).get("sectors") or {}
    best, w = "", 0.0
    for name, weight in sectors.items():
        name = exposure.norm_sector(name) or name
        if _num(weight, 0.0) > w:
            best, w = name, _num(weight, 0.0)
    return best or UNCLASSIFIED


def watch_rows(base, positions):
    """The watchlist as rows: the listing, its last price and day change from the
    quote the app keeps for it, and the holding it is when the book holds it too."""
    quotes = base.get("quotes") or {}
    exposures = base.get("exposures") or {}
    held = {}
    for p in positions:
        held.setdefault((p["symbol"], _s(p.get("exchange")).upper()), p)
    out = []
    for w in base.get("watchlist") or []:
        q = quotes.get(watch_quote_key(w["symbol"], w.get("exchange"))) or {}
        pos = held.get((w["symbol"], _s(w.get("exchange")).upper()))
        inst = instruments.find(w["symbol"], w.get("exchange"))
        crypto = _s(w.get("exchange")).upper() == "CRYPTO"
        rec = None if inst or crypto else exposures.get(watch_exposure_key(w["symbol"], w.get("exchange"), w.get("currency")))
        out.append({
            "symbol": w["symbol"], "exchange": inst["exchange"] if inst else "Crypto" if crypto else (w.get("exchange") or ""), "name": w.get("name") or "", "currency": "USD" if crypto else (w.get("currency") or ""),
            "last": _num(q.get("price"), None), "priceChange": _num(q.get("priceChange"), None), "percentChange": _num(q.get("percentChange"), None),
            "sector": instruments.KIND_LABEL.get(inst["kind"], inst["kind"]) if inst else "Digital assets" if crypto else dominant_sector(rec) if rec else UNCLASSIFIED,
            "kind": inst["kind"] if inst else "Crypto" if crypto else "Shares", "positionId": pos["id"] if pos else None,
        })
    return out


def heatmap_items(positions, exposures, cad):
    """One tile per symbol held: its market value in CAD summed over the accounts holding
    it, the quote's day change, and the sector its record gives most weight to (a coin is
    Digital assets, a contract counts under its underlying's record)."""
    out, by_key = [], {}
    for p in positions:
        v = cad(p["mv"], p["currency"])
        if not (v > 0):
            continue
        if p.get("kind") == "Crypto":
            sector = "Digital assets"
        elif p.get("kind") == "Options":
            under = _s(p.get("underlying") or "").upper()
            us, ca = exposure.SHARE_KEY + under + "::US", exposure.SHARE_KEY + under + ":"
            first, second = (us, ca) if _s(p.get("currency")).upper() == "USD" else (ca, us)
            sector = dominant_sector(exposures.get(first) or exposures.get(second))
        else:
            sector = dominant_sector(exposures.get(p.get("securityId")))
        key = (p["symbol"], _s(p.get("exchange")).upper())
        if key in by_key:
            by_key[key]["value"] += v
            continue
        by_key[key] = {"id": p["id"], "symbol": p["symbol"], "exchange": p.get("exchange") or "", "value": v, "percentChange": p.get("percentChange"), "sector": sector}
        out.append(by_key[key])
    return out


def news_text_key(headline):
    """A headline as one story: letters and digits only, one case, one space between words."""
    return " ".join(re.sub(r"[^a-z0-9]+", " ", _s(headline).lower()).split())


FRENCH_WORDS = re.compile(r"\b(annonce|annoncent|ses|du|des|une|pour|avec|sur|résultats|clôture|croissance|les|et|au|aux|dans|son|sa|le|la)\b")


def looks_french(headline):
    """A headline written in French: accented letters or French function words, two or more."""
    t = _s(headline).lower()
    return len(re.findall(r"[àâçéèêëîïôûùüÿœ]", t)) >= 2 or len(FRENCH_WORDS.findall(t)) >= 2


def _when_minutes(iso):
    try:
        return datetime.fromisoformat(_s(iso).replace("Z", "+00:00")).timestamp() / 60.0
    except ValueError:
        return None


def drop_translations(rows):
    """A release posted in French beside its English original (the same wire, a listing in
    common, within three hours) is one story: the English row stays, the French one goes."""
    keys = lambda r: {(t["symbol"], _s(t["exchange"]).upper()) for t in r["tags"]}
    out = []
    for r in rows:
        if looks_french(r["headline"]):
            tr, kr = _when_minutes(r["publishedAt"]), keys(r)
            twin = any(o is not r and not looks_french(o["headline"]) and o["source"] == r["source"] and (keys(o) & kr)
                       and tr is not None and _when_minutes(o["publishedAt"]) is not None and abs(_when_minutes(o["publishedAt"]) - tr) <= 180
                       for o in rows)
            if twin:
                continue
        out.append(r)
    return out


def news_rows(base, positions, watch):
    """Every item kept, newest first, each tagged with the listings it was read for: the
    symbol, whether the book holds it or watches it, and its day change. An item two
    listings share (a wire's own id) is one row with two tags."""
    # a listing is one listing whether the book names it QNC.TO or the watchlist QNC: the bare ticker and the venue
    lk = lambda symbol, exchange: (market.tmx_symbol(symbol), _s(exchange).upper())
    held = {}
    for p in positions:
        held.setdefault(lk(p["symbol"], p.get("exchange")), p)
    watched = {lk(w["symbol"], w.get("exchange")): w for w in watch}
    rows, by_id, by_text = [], {}, {}
    # one story is one row: the same wire id, or the same headline under another id (a
    # release carried by several wires, a story republished per symbol, an update)
    items = sorted(base.get("news") or [], key=lambda n: n.get("publishedAt") or "", reverse=True)
    for n in items:
        # the market feed's items carry no tag: they are the market's, not a listing's
        is_market = (_s(n["symbol"]), _s(n.get("exchange")).upper()) == (news.MARKET[0], news.MARKET[1])
        key = lk(n["symbol"], n.get("exchange"))
        p, w = held.get(key), watched.get(key)
        tag = None if is_market else {"symbol": key[0], "exchange": n.get("exchange") or "", "held": bool(p), "watched": bool(w),
                                      "percentChange": p.get("percentChange") if p else (w.get("percentChange") if w else None), "positionId": p["id"] if p else None}
        text = news_text_key(n.get("headline"))
        row = by_id.get(n["id"]) or (by_text.get(text) if text else None)
        if row:
            if is_market:
                row["market"] = True
            elif not any(lk(t["symbol"], t["exchange"]) == key for t in row["tags"]):
                row["tags"].append(tag)
            by_id[n["id"]] = row
            continue
        row = {"id": n["id"], "headline": n.get("headline") or "", "source": n.get("wire") or "", "url": n.get("url") or "", "publishedAt": n.get("publishedAt") or "",
               "market": is_market, "tags": [] if is_market else [tag]}
        by_id[n["id"]] = row
        if text:
            by_text[text] = row
        rows.append(row)
    rows.sort(key=lambda r: r["publishedAt"], reverse=True)
    return drop_translations(rows)


def markets_view(base, positions):
    fx = base["fx"]
    today = base["today"]
    cad = lambda amount, currency: to_cad(fx, amount, currency, today)
    watch = watch_rows(base, positions)
    universes = {k: [{"id": None, "symbol": r["symbol"], "name": r.get("name") or "", "value": r.get("value") or 0.0, "percentChange": r.get("percentChange"), "sector": r.get("sector") or UNCLASSIFIED, "country": r.get("country") or ""}
                     for r in rows] for k, rows in (base.get("universes") or {}).items()}
    directory = [{"symbol": r["symbol"], "label": instruments.label(r["symbol"]), "name": r["name"], "exchange": r["exchange"], "kind": r["kind"], "aliases": list(r["aliases"])} for r in instruments._rows()]
    return {"holdings": heatmap_items(positions, base.get("exposures") or {}, cad), "watchlist": watch, "news": news_rows(base, positions, watch), "universes": universes,
            "tiles": tile_rows(base), "instruments": directory}


def portfolio_view(base, f, positions):
    """The Portfolio tiles: CAD aggregates over the accounts in scope. Market value,
    cost basis and unrealized P&L come from the open positions in scope, converted
    at today's rate. Net asset value is the sum of Wealthsimple's net liquidation
    value per account, margin used the negative cash balances per currency, available
    margin Wealthsimple's buying power for margin accounts (the server asks only those:
    every self-directed account answers the query with its cash to buy with, which is
    not margin); each over the open accounts the filter has on, every open account when
    it has none, and None when no account in scope reports it."""
    fx = base["fx"]
    today = base["today"]
    cad = lambda amount, currency: to_cad(fx, amount, currency, today)
    names = f["lists"]["account"]
    # closed accounts hold nothing and count for nothing here
    accounts = [a for a in base["accounts"] if _s(a.get("status")).lower() != "closed" and (not names or a["name"] in names)]
    ids = {a["id"] for a in accounts}
    name_of = {a["id"]: a["name"] for a in accounts}
    mv = sum(cad(p["mv"] if not p["short"] else -p["mv"], p["currency"]) for p in positions)
    cost = sum(cad(abs(p["cost"]), p["currency"]) for p in positions)
    unreal = sum(cad(p["unreal"], p["currency"]) for p in positions)
    navs = [cad(a["nav"], a["currency"]) for a in accounts if a.get("nav") is not None]
    cash_ccy = base.get("cashCurrencies") or {}
    used = {}
    for b in base.get("balances") or []:
        aid = _s(b.get("accountId"))
        ccy = cash_ccy.get(_s(b.get("securityId")))
        q = _num(b.get("quantity"), 0.0)
        if aid in ids and ccy and q < 0:
            used[ccy] = used.get(ccy, 0.0) + (-q)
    margin_used = sum(cad(v, c) for c, v in used.items())
    # the positive cash balances, the other side of the same rows
    cash_by = {}
    for b in base.get("balances") or []:
        aid = _s(b.get("accountId"))
        ccy = cash_ccy.get(_s(b.get("securityId")))
        q = _num(b.get("quantity"), 0.0)
        if aid in ids and ccy and q > 0:
            cash_by[ccy] = cash_by.get(ccy, 0.0) + q
    cash = sum(cad(v, c) for c, v in cash_by.items())
    # the day's change: each quoted position's, summed, over what those positions were worth at the previous close
    quoted = [p for p in positions if p.get("dayChange") is not None]
    day_change = sum(cad(p["dayChange"], p["currency"]) for p in quoted) if quoted else None
    prev_value = (sum(cad(p["mv"] if not p["short"] else -p["mv"], p["currency"]) for p in quoted) - day_change) if quoted else 0.0
    # only a margin account's buying power is margin available; any other row is cash to buy with
    margin_ids = {a["id"] for a in accounts if "MARGIN" in _s(a.get("type")).upper()}
    avail = []
    unavailable = []
    for m in base.get("margin") or []:
        aid = _s(m.get("accountId"))
        if aid not in margin_ids:
            continue
        bp = _num(m.get("buyingPower"), None)
        if bp is None:
            unavailable.append(name_of.get(aid, aid))
        else:
            avail.append(cad(bp, m.get("currency") or "CAD"))
    alloc = []
    for p in positions:
        v = cad(p["mv"], p["currency"])
        if v > 0:
            alloc.append({"id": p["id"], "symbol": p["symbol"], "account": p["account"], "value": v})
    alloc.sort(key=lambda x: -x["value"])
    total = sum(x["value"] for x in alloc)
    for x in alloc:
        x["share"] = (x["value"] / total) if total else 0.0
    sectors, regions = exposure_slices(positions, base.get("exposures") or {}, cad)
    return {
        "allocation": alloc,
        "sectors": sectors,
        "regions": regions,
        "marketValue": mv,
        "costBasis": cost,
        "unrealized": unreal,
        "unrealizedPct": (unreal / cost) if cost else None,
        "positionCount": len(positions),
        "accountCount": len({p["account"] for p in positions}),
        "nav": sum(navs) if navs else None,
        "navAccounts": len(navs),
        "marginUsed": margin_used,
        "marginUsedBy": {c: round(v, 2) for c, v in sorted(used.items())},
        "marginUsedPct": (margin_used / mv) if mv else None,
        "availableMargin": sum(avail) if avail else None,
        "availableMarginUnavailable": sorted(unavailable),
        # the tiles a book without a margin account shows in the margin tiles' places
        "hasMargin": bool(margin_ids),
        "cash": cash,
        "cashPct": (cash / sum(navs)) if navs and sum(navs) else None,
        "dayChange": day_change,
        "dayChangePct": (day_change / prev_value) if quoted and prev_value else None,
    }


def position_matches(p, f):
    s = f["search"].upper()
    if s and s not in p["symbol"].upper() and s not in _s(p.get("name")).upper():
        return False
    L = f["lists"]
    if L["account"] and p["account"] not in L["account"]:
        return False
    if L["symbol"] and p["symbol"] not in L["symbol"] and p["underlying"] not in L["symbol"]:
        return False
    if L["kind"] and p["kind"] not in L["kind"]:
        return False
    if L["exchange"] and p["exchange"] not in L["exchange"]:
        return False
    return True


def metrics(trades):
    vals = [t["pnlCad"] for t in trades]
    wins = [v for v in vals if v > 0]
    losses = [v for v in vals if v < 0]
    be = [v for v in vals if v == 0]
    gw = sum(wins)
    gl = abs(sum(losses))
    n = len(vals)
    total = sum(vals)
    return {
        "realized": total,
        "count": n,
        "wins": len(wins),
        "losses": len(losses),
        "breakeven": len(be),
        "winRate": (len(wins) / n) if n else None,
        "grossWin": gw,
        "grossLoss": gl,
        "profitFactor": (gw / gl) if gl > 0 else (None if gw > 0 else 0.0),
        "profitFactorInfinite": gl == 0 and gw > 0,
        "expectancy": (total / n) if n else None,
        "avgWin": (gw / len(wins)) if wins else 0.0,
        "avgLoss": (-gl / len(losses)) if losses else 0.0,
        "fees": sum(t["feesCad"] for t in trades),
        "avgHold": (sum(t["holdDays"] for t in trades) / n) if n else None,
        "openCount": len([t for t in trades if t["status"] == "open"]),
    }


def by_symbol(trades):
    by = {}
    order = []
    for t in trades:
        k = t["underlying"]
        if k not in by:
            by[k] = {"symbol": k, "pnl": 0.0, "n": 0, "wins": 0, "hold": 0, "legs": 0, "tradeIds": []}
            order.append(k)
        g = by[k]
        g["pnl"] += t["pnlCad"]
        g["n"] += 1
        g["legs"] += t["legCount"]
        g["hold"] += t["holdDays"]
        g["tradeIds"].append(t["id"])
        if t["pnlCad"] > 0:
            g["wins"] += 1
    rows = []
    for k in order:
        g = by[k]
        rows.append(
            {
                "symbol": g["symbol"],
                "pnl": g["pnl"],
                "n": g["n"],
                "legs": g["legs"],
                "winRate": g["wins"] / g["n"] if g["n"] else 0.0,
                "avgHold": g["hold"] / g["n"] if g["n"] else 0,
                "tradeIds": g["tradeIds"],
            }
        )
    rows.sort(key=lambda r: r["pnl"], reverse=True)
    return rows


def month_label(key):
    return "%s '%s" % (MONTHS[int(key[5:7]) - 1], key[2:4])


def monthly(trades):
    by = {}
    for t in trades:
        k = t["exitDate"][:7]
        if not k or len(k) < 7:
            continue
        b = by.setdefault(k, {"key": k, "label": month_label(k), "value": 0.0, "count": 0, "tradeIds": []})
        b["value"] += t["pnlCad"]
        b["count"] += 1
        b["tradeIds"].append(t["id"])
    return [by[k] for k in sorted(by)]


def grade_buckets(trades):
    buckets = []
    for g in GRADES:
        rows = [t for t in trades if t["grade"] == g]
        buckets.append({"grade": g, "n": len(rows), "pnl": sum(t["pnlCad"] for t in rows), "tradeIds": [t["id"] for t in rows]})
    ungraded = [t for t in trades if not t["grade"]]
    return {"buckets": buckets, "ungraded": len(ungraded), "graded": len(trades) - len(ungraded)}


def review_queue(trades):
    out = []
    for t in trades:
        no_grade = not t["grade"]
        no_thesis = not _s(t.get("thesis")).strip()
        if not (no_grade or no_thesis):
            continue
        out.append(
            {
                "id": t["id"],
                "symbol": t["symbol"],
                "date": t["exitDate"],
                "pnl": t["pnlCad"],
                "currency": "CAD",
                "missing": "no grade or thesis" if (no_grade and no_thesis) else ("no grade" if no_grade else "no thesis"),
            }
        )
    out.sort(key=lambda r: r["date"], reverse=True)
    return out


_SCHEDULES = (52, 26, 24, 12, 6, 4, 2, 1)


def payments_per_year(dates):
    """Verified payment frequency from actual payment dates (any order).
    Only the most recent gaps count (the last three), so a fund that changes
    its schedule is re-read after two payments at the new cadence. Payments
    on the same day count once. None when fewer than two distinct dates."""
    days = sorted({_s(d)[:10] for d in dates if _s(d)[:10]})
    if len(days) < 2:
        return None
    gaps = [days_between(a, b) for a, b in zip(days, days[1:])]
    gaps = [g for g in gaps if g > 0][-3:]
    if not gaps:
        return None
    gaps.sort()
    median = gaps[len(gaps) // 2]
    per_year = 365.25 / median
    return min(_SCHEDULES, key=lambda s: abs(s - per_year))


def cashflow_view(base, f, positions_all, margin_used=0.0, has_margin=True):
    today = base["today"]
    L = f["lists"]
    accts = L["account"]
    search = f["search"].upper()

    def in_scope(r):
        if accts and r["account"] not in accts:
            return False
        if search and search not in r["symbol"].upper():
            return False
        if L["symbol"] and r["symbol"] not in L["symbol"]:
            return False
        return in_date_scope(f, today, r["date"])

    everything = [r for r in base["cashflow"] if in_scope(r)]
    recs = [r for r in everything if r["kind"] in ("Dividend",)]
    skipped = [k for k in ("grade", "tag", "kind", "exchange", "side", "result") if L[k]]
    skipped += [k for k, r in f["ranges"].items() if r["v"] is not None]

    keys = []
    bucket = {}
    if recs:
        months_seen = sorted({r["date"][:7] for r in recs})
        first, last = months_seen[0], months_seen[-1]
        # the chart runs to the current month (or the end of the date filter), with
        # an empty bar for a month that has not paid yet
        end_day = today
        bounds = date_bounds(f, today)
        if bounds:
            end_day = min(bounds[1], today)
        elif f["years"]:
            end_day = min(max(f["years"]) + "-12-31", today)
        last = max(last, end_day[:7])
        y, m = int(first[:4]), int(first[5:7])
        while True:
            k = "%04d-%02d" % (y, m)
            if k > last:
                break
            keys.append(k)
            bucket[k] = {"sum": 0.0, "n": 0}
            m += 1
            if m > 12:
                m = 1
                y += 1
    for r in recs:
        k = r["date"][:7]
        if k in bucket:
            bucket[k]["sum"] += r["amountCad"]
            bucket[k]["n"] += 1
    months = [{"key": k, "label": month_label(k), "value": bucket[k]["sum"], "count": bucket[k]["n"]} for k in keys]

    payers = {r["symbol"] for r in base["cashflow"] if r["kind"] == "Dividend"}
    held = [p for p in positions_all if p["symbol"] in payers and not p["short"]]
    held = [p for p in held if (not accts or p["account"] in accts) and (not search or search in p["symbol"].upper())]
    for_yoc = [r for r in base["cashflow"] if r["kind"] == "Dividend" and (not accts or r["account"] in accts) and (not search or search in r["symbol"].upper())]
    last_rec = recs[0]["date"] if recs else today
    cut_dt = date.fromisoformat(last_rec)
    cm = cut_dt.month - 11
    cy = cut_dt.year
    while cm <= 0:
        cm += 12
        cy -= 1
    cut = "%04d-%02d" % (cy, cm)
    this_year = today[:4]

    def sum_for(sym, pred):
        return sum(r["amountCad"] for r in for_yoc if r["symbol"] == sym and pred(r))

    public = base.get("distributions") or {}
    quotes = base.get("quotes") or {}

    def rate_for(sym):
        # Preferred: the fund's own declared record (TMX Money): the latest
        # distribution that has gone ex, and payments per year from the gaps
        # between its recent ex-dates, so a schedule change shows at once.
        declared = [d for d in public.get(sym, []) if d["exDate"] <= today]
        if declared:
            declared.sort(key=lambda d: d["exDate"], reverse=True)
            per = declared[0]["amount"]
            freq = payments_per_year([d["exDate"] for d in public.get(sym, [])])
            if per and freq:
                return {"per": per, "freq": freq, "annual": per * freq, "verified": True, "source": "declared"}
        # Otherwise this holding's own payment rows.
        rs = sorted([r for r in for_yoc if r["symbol"] == sym and r["per"]], key=lambda r: r["date"], reverse=True)
        if not rs:
            return None
        per = rs[0]["per"]
        if not per:
            return None
        freq = payments_per_year([r["date"] for r in for_yoc if r["symbol"] == sym])
        verified = freq is not None
        if not verified:
            freq = 12
        return {"per": per, "freq": freq, "annual": per * freq, "verified": verified, "source": "payments"}

    def distribution_dates(sym):
        """(ex-date, pay date, ex passed, pay passed): the next distribution still
        to be paid, whether or not it has gone ex, else the last known one. The
        fund's declared record first; failing that the ex-date TMX reports on the
        quote and the last payment received. A date is 'passed' once it is
        before today."""
        recs_ = sorted(public.get(sym, []), key=lambda d: (_s(d.get("payDate"))[:10] or d["exDate"], d["exDate"]))
        unpaid = [d for d in recs_ if (_s(d.get("payDate"))[:10] or d["exDate"]) >= today]
        pick = unpaid[0] if unpaid else (recs_[-1] if recs_ else None)
        if pick:
            ex, pay = pick["exDate"], _s(pick.get("payDate"))[:10]
        else:
            q = quotes.get(sym) or {}
            ex = _s(q.get("exDividendDate"))[:10]
            paid = sorted(r["date"] for r in for_yoc if r["symbol"] == sym)
            pay = paid[-1] if paid else ""
        return ex, pay, bool(ex and ex < today), bool(pay and pay < today)

    def last_price(p):
        q = quotes.get(p["symbol"]) or {}
        if not quote_fits(q, p.get("kind")):
            q = {}
        px = _num(q.get("price"), None)
        if px and px > 0:
            return px, "close"
        return p["last"], "fill"

    holdings = []
    for p in held:
        r = rate_for(p["symbol"])
        basis = p["cost"]
        avg = p["avg"]
        last_px, price_source = last_price(p)
        holdings.append(
            {
                "id": p["id"],
                "symbol": p["symbol"],
                "account": p["account"],
                "qty": p["qty"],
                "per": r["per"] if r else None,
                "freq": r["freq"] if r else None,
                "freqVerified": bool(r and r["verified"]),
                "rateSource": r["source"] if r else "",
                "cost": basis,
                "avg": avg,
                "last": last_px,
                "priceSource": price_source,
                "ytd": sum_for(p["symbol"], lambda x: x["date"][:4] == this_year),
                "ttm": sum_for(p["symbol"], lambda x: x["date"][:7] >= cut),
                "all": sum_for(p["symbol"], lambda x: True),
                "nextExDate": distribution_dates(p["symbol"])[0],
                "nextPayDate": distribution_dates(p["symbol"])[1],
                "exPast": distribution_dates(p["symbol"])[2],
                "payPast": distribution_dates(p["symbol"])[3],
                "yob": (r["per"] * p["qty"]) if r else None,
                "annual": (r["annual"] * p["qty"]) if (r and r["annual"] is not None) else None,
                "yoc": (r["annual"] / avg) if (r and r["annual"] is not None and avg) else None,
                "currentYield": (r["annual"] / last_px) if (r and r["annual"] is not None and last_px) else None,
            }
        )
    verified = [h for h in holdings if h["annual"] is not None]
    basis_all = sum(h["cost"] for h in verified)
    earned_all = sum(h["ttm"] for h in verified)
    annual_all = sum(h["annual"] for h in verified)
    total = sum(r["amountCad"] for r in recs)
    this_yr = int(this_year)
    tiles = []
    for y in (this_yr - 2, this_yr - 1, this_yr):
        rs = [r for r in recs if r["date"][:4] == str(y)]
        sm = sum(r["amountCad"] for r in rs)
        paid = len([k for k in keys if k[:4] == str(y) and bucket[k]["n"] > 0]) or 1
        tiles.append({"label": ("%d YTD" % y) if y == this_yr else str(y), "total": sm, "perMonth": sm / paid, "count": len(rs)})
    months_in_scope = len([k for k in keys if bucket[k]["n"] > 0]) or 1
    tiles.append({"label": "All time", "total": total, "perMonth": total / months_in_scope, "count": len(recs)})
    if has_margin:
        # margin used is the Portfolio tab's figure; under it the average margin interest per charged month
        charges = [r for r in everything if r["kind"] == "Interest charge"]
        charge_months = len({r["date"][:7] for r in charges})
        charged = sum(-r["amountCad"] for r in charges)
        tiles.append({"label": "Margin used", "marginUsed": margin_used, "interestPerMonth": (charged / charge_months) if charge_months else 0.0, "interestMonths": charge_months})
    else:
        # without a margin account: the trailing twelve months, averaged over the months that paid
        since = shift_date(today, -365)
        window = [r for r in recs if since < r["date"] <= today]
        sm = sum(r["amountCad"] for r in window)
        paid = len({r["date"][:7] for r in window}) or 1
        tiles.append({"label": "Last 12 months", "total": sm, "perMonth": sm / paid, "count": len(window)})
    tiles.append(
        {
            "label": "Yield on cost",
            "yield": (annual_all / basis_all) if basis_all else None,
            "projected": annual_all / 12,
            "earned": earned_all,
            "book": basis_all,
        }
    )
    other = [r for r in everything if r["kind"] != "Dividend"]
    return {
        "tiles": tiles,
        "months": months,
        "holdings": holdings,
        "rows": recs,
        "other": other,
        "total": total,
        "count": len(recs),
        "skippedFilters": skipped,
        "interest": sum(r["amountCad"] for r in other if r["kind"] == "Interest"),
        "withholding": sum(r["amountCad"] for r in other if r["kind"] == "Withholding tax"),
    }


def build_view(base, filters=None):
    f = clean_filters(filters)
    today = base["today"]
    trades_all = base["trades"]
    trades = [t for t in trades_all if trade_matches(t, f, today)]
    positions_all = base["positions"]
    positions = [p for p in positions_all if position_matches(p, f)]

    accts = f["lists"]["account"]
    if len(accts) == 1 and accts[0] in base["equityByAccount"]:
        series = base["equityByAccount"][accts[0]]
        series_label = accts[0]
    else:
        series = base["equity"]
        series_label = "All accounts"
    bench_key = f["benchmark"]
    years = yearly_returns(series, (base.get("benchmarks") or {}).get(bench_key) or {}, today)
    ann = annualized(years)
    dd = drawdown(series)
    bounds = date_bounds(f, today)
    portfolio = portfolio_view(base, f, positions)
    shown = series
    if bounds:
        shown = [p for p in series if bounds[0] <= p["d"] <= bounds[1]]
    elif f["years"]:
        shown = [p for p in series if p["d"][:4] in f["years"]]
    if shown:
        peak = max(p["v"] for p in shown)
        first_idx = next((i for i, p in enumerate(shown) if p["v"] > peak * 0.01), 0)
        shown = shown[first_idx:]

    tags = sorted({tag for t in trades_all for tag in t["tags"]})
    symbols = sorted({t["symbol"] for t in trades_all} | {p["symbol"] for p in positions_all})
    # what the ⌘K list shows beside each symbol: its name, exchange and kind, from the
    # rows that carry it (a name that is only the symbol again counts as none)
    listings = {}
    for r in list(trades_all) + list(positions_all):
        cur = listings.setdefault(r["symbol"], {"name": "", "exchange": "", "kind": _s(r.get("kind")), "currency": _s(r.get("currency"))})
        if not cur["name"] and _s(r.get("name")) and _s(r.get("name")) != r["symbol"]:
            cur["name"] = _s(r.get("name"))
        if not cur["exchange"] and _s(r.get("exchange")):
            cur["exchange"] = _s(r.get("exchange"))
    accounts = sorted({t["account"] for t in trades_all} | {p["account"] for p in positions_all} | {r["account"] for r in base["cashflow"]})
    exchanges = sorted({t["exchange"] for t in trades_all if t["exchange"]} | {p["exchange"] for p in positions_all if p["exchange"]})
    kinds = [k for k in KINDS if any(t["kind"] == k for t in trades_all) or any(p["kind"] == k for p in positions_all)]
    year_options = sorted({t["exitDate"][:4] for t in trades_all if t["exitDate"]}, reverse=True)

    book = sum(abs(p["cost"]) for p in positions)
    mv = sum(p["mv"] if not p["short"] else -p["mv"] for p in positions)
    unreal = sum(p["unreal"] for p in positions)

    return {
        "ok": True,
        "generated": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "today": today,
        "syncedAt": base["syncedAt"],
        "currency": "CAD",
        "market": {"fxLast": base["fxLast"], "benchmarkLast": base["benchmarkLast"]},
        "filters": f,
        "options": {
            "accounts": accounts,
            "symbols": symbols,
            "listings": listings,
            "tags": tags,
            "exchanges": exchanges,
            "kinds": kinds,
            "grades": list(GRADES) + ["Ungraded"],
            "sides": ["SELL", "COVER"],
            "results": ["Winners", "Losers", "Breakeven"],
            "years": year_options,
        },
        "kpi": metrics(trades),
        "equity": {
            "label": series_label,
            "series": shown,
            "drawdown": dd,
            "annualized": ann,
        },
        "years": years,
        "benchmark": {"key": bench_key, "label": BENCHMARK_LABELS[bench_key]},
        "monthly": monthly(trades),
        "bySymbol": by_symbol(trades),
        "grades": grade_buckets(trades),
        "queue": review_queue(trades),
        "trades": trades,
        "tradeCount": len(trades),
        "tradeTotal": len(trades_all),
        "positions": positions,
        "positionsSummary": {"count": len(positions), "book": book, "mv": mv, "unreal": unreal},
        "portfolio": portfolio,
        "markets": markets_view(base, positions),
        "cashflow": cashflow_view(base, f, positions_all, portfolio["marginUsed"], portfolio["hasMargin"]),
        "unmatched": base["unmatched"],
        "accounts": base["accounts"],
        "activityCount": base["activityCount"],
    }


# --------------------------------------------------------------------------
# cache + entry points used by the HTTP server
# --------------------------------------------------------------------------

_cache_lock = threading.Lock()
_cache = {"version": None, "core": None, "base": None, "inputs": None}
# The matched book outlives the base: a quote tick changes the data version every
# minute, but the FIFO match only changes with the activity rows, the securities
# or the day, so it is kept across ticks and the activity rows are not re-read.
_book = {"key": None, "book": None}


def base_model(force=False):
    # today's date is part of the key: YTD tiles, the current year's return and
    # anything else measured "to today" must roll over at midnight even when
    # nothing in the database has changed
    today = today_local()
    full, core = store.versions()
    version = full + "|" + today
    core_key = core + "|" + today
    with _cache_lock:
        if not force and _cache["base"] is not None and _cache["version"] == version:
            return _cache["base"]
        marked = None
        if not force and _cache["base"] is not None and _cache["core"] == core_key:
            marked = (_cache["base"], _cache["inputs"])
    if marked is not None:
        return _remark(marked[0], marked[1], today, version, core_key)
    book_key = store.book_version() + "|" + today
    with _cache_lock:
        book = _book["book"] if not force and _book["key"] == book_key else None
    snapshot = store.snapshot(activities=book is None)
    market = store.market_data()
    journal = store.journal()
    if book is None:
        book = build_book(snapshot, today)
    if not journal and snapshot.get("notes"):
        probe = build_base(snapshot, market, {}, today, book=book)
        migrated = migrate_legacy_notes(probe["closed"], snapshot.get("tradeGroups"), snapshot.get("notes"))
        if migrated:
            journal = store.save_journal(migrated)
            version = store.data_version() + "|" + today
    base = build_base(snapshot, market, journal, today, book=book)
    with _cache_lock:
        _cache["version"] = version
        _cache["core"] = core_key
        _cache["base"] = base
        _cache["inputs"] = {
            "accounts": snapshot.get("accounts") or [],
            "balances": snapshot.get("balances") or [],
            "journal": journal,
        }
        _book["key"] = book_key
        _book["book"] = book
    return base


def _remark(base, inputs, today, version, core_key):
    """A price tick and nothing else: mark the open positions at the new quotes
    and keep the rest of the model as it stands. Re-matching the whole book,
    collapsing every closed trade and walking the cashflow again for a price
    that moved a cent was what made a modest machine unusable."""
    quotes = (store.market_data().get("quotes")) or {}
    fresh = dict(base)
    fresh["quotes"] = quotes
    fresh["positions"] = build_positions(
        base["openLots"],
        base["lastPrices"],
        inputs["balances"],
        inputs["accounts"],
        base["securities"],
        inputs["journal"],
        today,
        quotes,
        base["actsById"],
    )
    with _cache_lock:
        _cache["version"] = version
        _cache["core"] = core_key
        _cache["base"] = fresh
    return fresh


# What a trade or holding row carries only when it is the one open on the page:
# every leg and every fill of every trade is most of the model's bytes, and the
# lists never read them.
DETAIL_KEYS = ("legs", "fills")


def _without_detail(row):
    return {k: v for k, v in row.items() if k not in DETAIL_KEYS}


def slim(view, detail=None):
    """The view for the page: legs and fills only on the trade or holding `detail` names."""
    out = dict(view)
    for key in ("trades", "positions"):
        out[key] = [r if detail and r.get("id") == detail else _without_detail(r) for r in view.get(key) or []]
    return out


def trade_detail(trade_id, base=None):
    """The legs and fills of one trade or holding, by id; None when there is none."""
    base = base or base_model()
    for key in ("trades", "positions"):
        for r in base[key]:
            if r.get("id") == trade_id:
                return {"id": trade_id, "legs": list(r.get("legs") or []), "fills": list(r.get("fills") or [])}
    return None


def view(filters=None, detail=None):
    return slim(build_view(base_model(), filters), detail)


def held_symbols(base=None):
    """Every held instrument, with what a quote source needs to price it."""
    base = base or base_model()
    out = []
    seen = set()
    for p in base["positions"]:
        if p["symbol"] in seen:
            continue
        seen.add(p["symbol"])
        out.append({"symbol": p["symbol"], "exchange": p["exchange"], "currency": p["currency"], "kind": p["kind"]})
    return out


def intraday_archive_symbols(base=None, since=None):
    """Instruments whose intraday bars are worth keeping: every symbol traded or
    held since `since` (a date), with the earliest date bars are wanted from."""
    base = base or base_model()
    since = _s(since)[:10] or shift_date(base["today"], -365)
    out = {}
    def want(rec, start):
        key = rec["symbol"]
        cur = out.get(key)
        if cur is None or start < cur["start"]:
            out[key] = dict(rec, start=start)
    def charted(rec):
        # an option trade is charted on its underlying, so that is what gets kept
        if rec["kind"] == "Options":
            under = underlying_symbol(rec["symbol"])
            if under and under != "—":
                return {"symbol": under, "exchange": rec["exchange"], "currency": rec["currency"], "kind": "Shares"}
        return rec
    for t in base["trades"]:
        if t["exitDate"] >= since:
            want(charted({"symbol": t["symbol"], "exchange": t["exchange"], "currency": t["currency"], "kind": t["kind"]}), max(t["entryDate"], since))
    for p in base["positions"]:
        want(charted({"symbol": p["symbol"], "exchange": p["exchange"], "currency": p["currency"], "kind": p["kind"]}), max(_s(p.get("opened")) or since, since))
    return [out[k] for k in sorted(out)]


def payer_symbols(base=None):
    """Held positions that have paid a distribution: what the public
    distribution feed is refreshed for."""
    base = base or base_model()
    payers = {r["symbol"] for r in base["cashflow"] if r["kind"] == "Dividend"}
    out = []
    seen = set()
    for p in base["positions"]:
        if p["symbol"] in payers and p["symbol"] not in seen and not p["short"]:
            seen.add(p["symbol"])
            out.append({"symbol": p["symbol"], "exchange": p["exchange"] if p["exchange"] != "Crypto" else "", "currency": p["currency"]})
    return out


def apply_journal(entries):
    """Push saved journal entries into the cached model without rebuilding it."""
    with _cache_lock:
        base = _cache["base"]
        if base is None:
            return
        entries = entries or {}
        for t in base["trades"]:
            e = entries.get(t["id"]) or {}
            t["grade"] = e.get("grade", "")
            t["thesis"] = e.get("thesis", "")
            t["tags"] = list(e.get("tags", []))
        for p in base["positions"]:
            e = entries.get(p["id"]) or {}
            p["grade"] = e.get("grade", "")
            p["thesis"] = e.get("thesis", "")
            p["tags"] = list(e.get("tags", []))
        # the next price tick marks the positions again from these inputs: it
        # must carry the journal that was just written, not the one before it
        if _cache["inputs"] is not None:
            _cache["inputs"]["journal"] = entries
        full, core = store.versions()
        _cache["version"] = full
        _cache["core"] = core + "|" + _s(base.get("today"))


def invalidate(book=False):
    """Forget the derived model. The FIFO match is kept: `_book` carries the
    fingerprint of the rows it was built from, so the next build rebuilds it
    exactly when the activities or the securities have changed. A quote, a
    headline or a heatmap tile changes none of those, and re-matching the whole
    book for one of them was the largest repeated cost on a slow machine."""
    with _cache_lock:
        _cache["version"] = None
        _cache["core"] = None
        _cache["base"] = None
        _cache["inputs"] = None
        if book:
            _book["key"] = None
            _book["book"] = None
