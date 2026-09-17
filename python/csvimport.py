"""CSV import and folder watching.

Three layouts are recognised (ported from the classic page's parser):

- canonical: a Wealthsimple activities export with transaction_date,
  activity_type, activity_sub_type, quantity, unit_price, net_cash_amount ...
- statement: a statement export with date, transaction, description, amount
  (fills are read out of the description text)
- legacy:    Date / Action / Symbol / Quantity / Price / Amount

Rows become local activities (never a Wealthsimple canonical id) and go
through store.merge_local_rows, which drops rows already stored. A watched
folder is scanned by the server itself: top-level .csv files, re-read only
when their size or modification time changes.
"""

from __future__ import annotations

import csv
import io
import json
import os
import re
import uuid
from datetime import datetime, timezone

import model
import store

WATCH_META = "watch_folder"
WATCH_FILES_META = "watch_files"
WATCH_LAST_META = "watch_last"

_MONTHS = {"jan": "01", "feb": "02", "mar": "03", "apr": "04", "may": "05", "jun": "06", "jul": "07", "aug": "08", "sep": "09", "oct": "10", "nov": "11", "dec": "12"}


def _s(v):
    return "" if v is None else str(v)


def normalize_header(h):
    s = _s(h).replace("﻿", "").strip().strip("\"'").strip().lower()
    return re.sub(r"[\s\-]+", "_", s)


def parse_number(raw):
    if raw is None:
        return 0.0
    s = _s(raw).strip()
    if not s or s in ("-", "—") or s.lower() == "n/a":
        return 0.0
    paren = s.startswith("(") and s.endswith(")")
    s = s.replace("(", "").replace(")", "")
    s = re.sub(r"[$£€,\s]|CAD|USD|cad|usd", "", s)
    if not s:
        return 0.0
    try:
        n = float(s)
    except ValueError:
        return 0.0
    return -abs(n) if paren else n


def parse_date(raw):
    s = _s(raw).strip()
    if not s:
        return ""
    m = re.match(r"^(\d{4})-(\d{2})-(\d{2})(?:[T\s].*)?$", s)
    if m:
        return "%s-%s-%s" % m.groups()
    m = re.match(r"^(\d{4})[/.](\d{1,2})[/.](\d{1,2})(?:\s.*)?$", s)
    if m:
        return "%s-%02d-%02d" % (m.group(1), int(m.group(2)), int(m.group(3)))
    m = re.match(r"^(\d{1,2})[- ]([A-Za-z]{3})[- ](\d{4})$", s)
    if m and m.group(2).lower() in _MONTHS:
        return "%s-%s-%02d" % (m.group(3), _MONTHS[m.group(2).lower()], int(m.group(1)))
    m = re.match(r"^([A-Za-z]{3})[- ](\d{1,2}),?[- ](\d{4})$", s)
    if m and m.group(1).lower() in _MONTHS:
        return "%s-%s-%02d" % (m.group(3), _MONTHS[m.group(1).lower()], int(m.group(2)))
    m = re.match(r"^(\d{1,2})[/\-.](\d{1,2})[/\-.](\d{4})(?:\s.*)?$", s)
    if m:
        a, b, y = int(m.group(1)), int(m.group(2)), m.group(3)
        if a > 12 and b <= 12:
            return "%s-%02d-%02d" % (y, b, a)
        return "%s-%02d-%02d" % (y, a, b)
    if re.match(r"^\d{4,6}(\.\d+)?$", s):
        serial = float(s)
        if 20000 < serial < 80000:
            from datetime import date, timedelta

            return (date(1899, 12, 30) + timedelta(days=int(round(serial)))).isoformat()
    return ""


def is_footer_line(text):
    return bool(re.match(r"^\s*as of\s+\d{4}-\d{2}-\d{2}", _s(text), re.I))


def detect_format(headers):
    norms = {normalize_header(h) for h in headers}
    canon_hints = ("transaction_date", "activity_type", "activity_sub_type", "net_cash_amount", "unit_price")
    legacy_hints = ("date", "action", "symbol", "quantity", "price", "amount")
    canon_hits = sum(1 for h in canon_hints if h in norms)
    legacy_hits = sum(1 for h in legacy_hints if h in norms)
    if canon_hits >= 3:
        return "canonical"
    if "transaction_date" in norms or "activity_type" in norms:
        return "canonical"
    if {"date", "transaction", "description", "amount"} <= norms:
        return "statement"
    if legacy_hits >= 5 and ("action" in norms or "date" in norms):
        return "legacy"
    if "action" in norms and "date" in norms:
        return "legacy"
    return "unknown"


def _compact_lower(s):
    return re.sub(r"[\s_\-]", "", _s(s).strip().lower())


def categorize(activity_type, activity_sub_type):
    t = _compact_lower(activity_type)
    s = _compact_lower(activity_sub_type)
    blob = t + " " + s
    if t in ("fxexchange", "fx") or s == "fxexchange" or "fxexchange" in blob:
        return "fx"
    if any(k in t or k in s for k in ("expir", "exercise", "assign")):
        return "option_event"
    if t == "trade" or s in ("buy", "sell"):
        return "trade"
    if "dividend" in t or "dividend" in s:
        return "dividend"
    if "deposit" in t or "deposit" in s:
        return "deposit"
    if "withdraw" in t or "withdraw" in s:
        return "withdrawal"
    if "interest" in t or "interest" in s:
        return "interest"
    if "fee" in t or "fee" in s:
        return "fee"
    if "transfer" in t or "transfer" in s:
        return "transfer"
    return "other"


def extract_instrument(description):
    text = _s(description).strip()
    if not text:
        return "", ""
    colon = text.find(":")
    dash_re = re.compile(r"^([A-Za-z][A-Za-z0-9.\-]{0,20})\s+-\s+(.+)$")
    if colon < 0:
        m = dash_re.match(text)
        return (m.group(1).upper(), m.group(2).strip()) if m else ("", "")
    left = text[:colon].strip()
    m = dash_re.match(left)
    if m:
        return m.group(1).upper(), m.group(2).strip()
    if re.search(r"\s", left):
        sym = re.sub(r"\s+", " ", left).upper()
        return sym, sym
    if re.match(r"^[A-Za-z][A-Za-z0-9.\-]{0,20}$", left):
        return left.upper(), left.upper()
    return "", ""


def parse_statement_description(description):
    desc = _s(description)
    symbol, name = extract_instrument(desc)
    quantity = 0.0
    unit_price = 0.0
    executed_at = ""
    fill_parsed = False
    contract_signed = 0.0
    shares_signed = 0.0
    shares = re.search(r"(-?[\d,]+(?:\.\d+)?)\s+shares?\b", desc, re.I)
    contracts = re.search(r"(-?[\d,]+(?:\.\d+)?)\s+contracts?\b", desc, re.I)
    price = re.search(r"\bat\s+\$?([\d,]+(?:\.\d+)?)\s+per\s+share\b", desc, re.I)
    if shares:
        shares_signed = parse_number(shares.group(1))
        quantity = abs(shares_signed)
        fill_parsed = quantity > 0
    elif contracts:
        contract_signed = parse_number(contracts.group(1))
        quantity = abs(contract_signed)
        fill_parsed = quantity > 0
    if price:
        unit_price = parse_number(price.group(1))
    exe = re.search(r"\(\s*executed at\s+(\d{4}-\d{2}-\d{2})\s*\)", desc, re.I)
    if exe:
        executed_at = parse_date(exe.group(1))
    return {"symbol": symbol, "name": name, "quantity": quantity, "unitPrice": unit_price, "executedAt": executed_at, "fillParsed": fill_parsed, "contractSigned": contract_signed, "sharesSigned": shares_signed}


def map_statement_type(code, description):
    raw = _s(code).strip()
    c = re.sub(r"[\s_\-]", "", raw.upper())
    blob = (c + " " + _s(description)).lower()
    if "EXPIR" in c or c in ("ASSIGN", "ASSIGNMENT", "EXERCISE"):
        return raw, raw.upper(), "option_event"
    if c in ("LOAN", "RECALL"):
        return raw or c, c, "other"
    if c in ("STKDIS", "STKDIV", "SPIN", "SPINOFF"):
        return raw or "STKDIS", "STKDIS", "trade"
    if c in ("ROC", "RETURNOFCAPITAL"):
        return raw or "ROC", "ROC", "other"
    if c in ("DIV", "DIVIDEND") or "DIVIDEND" in c:
        return "Dividend", raw or "DIV", "dividend"
    if c in ("CONT", "CONTRIBUTION") or "CONTRIB" in c:
        return "Deposit", raw or "CONT", "deposit"
    if c in ("WD", "WITHDRAWAL") or "WITHDRAW" in c:
        return "Withdrawal", raw or "WD", "withdrawal"
    if c in ("INTCHARGED", "INTPAID", "INTEREST") or c.startswith("INT"):
        return "Interest", raw or "INTEREST", "interest"
    if c in ("TRFOUT", "TRFIN", "TRANSFER") or c.startswith("TRF") or "TRANSFER" in c:
        return "Transfer", raw or "TRANSFER", "transfer"
    if c in ("FXCONVERSION", "FX", "CONVERT") or "FX" in c or "CONVERT" in c:
        return "FxExchange", raw or "FX", "fx"
    if c in ("FEE", "FCHRG", "COMM") or "FEE" in c or "FCHRG" in c:
        return "Fee", raw or "FEE", "fee"
    if c in ("BUY", "SELL"):
        return "Trade", c, "trade"
    if "SELL" in c:
        return raw or "Trade", "SELL", "trade"
    if "BUY" in c:
        return raw or "Trade", "BUY", "trade"
    if re.search(r"\breturn of capital\b|\broc\b", blob):
        return raw or "ROC", "ROC", "other"
    if re.search(r"\b(bought|sold|buy|sell)\b", blob) and not re.search(r"\bfx\b", blob) and not re.search(r"conversion|convert", blob):
        return "Trade", ("SELL" if re.search(r"\bsell|sold\b", blob) else "BUY"), "trade"
    if re.search(r"dividend|distribution", blob):
        return "Dividend", raw or "DIV", "dividend"
    if "interest" in blob:
        return "Interest", raw or "INTEREST", "interest"
    if re.search(r"transfer|trfout|trfin", blob):
        return "Transfer", raw or "TRANSFER", "transfer"
    if re.search(r"\bfx\b|conversion|convert", blob):
        return "FxExchange", raw or "FX", "fx"
    if re.search(r"\bfee\b|commission|fchrg", blob):
        return "Fee", raw or "FEE", "fee"
    if "deposit" in blob:
        return "Deposit", raw or "DEPOSIT", "deposit"
    if "withdraw" in blob:
        return "Withdrawal", raw or "WITHDRAWAL", "withdrawal"
    return raw or "Unknown", raw.upper(), categorize(raw, description)


def book_id_from_file_name(name):
    n = _s(name).split("/")[-1]
    m = re.search(r"([A-Z0-9]{8,}(?:CAD|USD))-\d{4}-\d{2}-\d{2}", n, re.I) or re.search(r"([A-Z0-9]{8,}(?:CAD|USD))", n, re.I)
    return m.group(1).upper() if m else n


def _pick(row, *keys):
    for k in keys:
        v = row.get(k)
        if v is not None and _s(v).strip():
            return _s(v).strip()
    return ""


def map_statement(row, book_id):
    settlement = parse_date(_pick(row, "date", "settlement_date", "transaction_date"))
    if not settlement:
        return None, None
    code = _pick(row, "transaction", "activity_type", "type", "action")
    description = _pick(row, "description", "memo", "details")
    parsed = parse_statement_description(description)
    activity_type, sub, category = map_statement_type(code, description)
    currency = (_pick(row, "currency", "ccy") or "CAD").upper()
    key_ccy = re.search(r"\b(USD|CAD)\b", parsed["symbol"], re.I)
    if key_ccy and currency not in ("CAD", "USD"):
        currency = key_ccy.group(1).upper()
    bal_raw = _pick(row, "balance")
    balance = None if bal_raw == "" else parse_number(bal_raw)
    net_cash = parse_number(_pick(row, "amount", "net_cash_amount", "net_amount"))
    compact_code = re.sub(r"[\s_\-]", "", code.upper())
    stk = compact_code in ("STKDIS", "STKDIV", "SPIN", "SPINOFF")
    if stk:
        activity_type, category = "STKDIS", "trade"
        sub = "SELL" if parsed["sharesSigned"] < 0 else "BUY"
    if parsed["fillParsed"] and category == "other" and compact_code not in ("LOAN", "RECALL"):
        sub = "BUY" if net_cash < 0 else "SELL"
        activity_type = activity_type if activity_type and activity_type != "Unknown" else "Trade"
        category = "trade"
    quantity = parsed["quantity"]
    unit_price = parsed["unitPrice"]
    if stk:
        unit_price = 0.0
    if parsed["fillParsed"] and unit_price == 0 and parsed["quantity"] > 0 and not stk:
        # the statement amount is full cash; options are premium x 100
        denom = parsed["quantity"] * model.option_multiplier(parsed["symbol"])
        unit_price = abs(net_cash) / denom if denom > 0 else 0.0
    if category == "option_event" and parsed["fillParsed"]:
        if parsed["contractSigned"] < 0:
            sub = "BUY"
        elif parsed["contractSigned"] > 0:
            sub = "SELL"
    if sub == "SELL":
        quantity = -abs(quantity)
    elif sub == "BUY":
        quantity = abs(quantity)
    issue = None
    if category == "trade" and sub in ("BUY", "SELL") and (not parsed["fillParsed"] or parsed["quantity"] == 0):
        issue = "Could not parse quantity/price from description for " + sub
    transaction_date = parsed["executedAt"] or settlement
    return {
        "id": str(uuid.uuid4()),
        "occurredAt": transaction_date,
        "transactionDate": transaction_date,
        "settlementDate": settlement,
        "accountId": "",
        "bookId": _s(book_id).strip(),
        "accountType": "",
        "activityType": activity_type,
        "activitySubType": sub,
        "description": description,
        "direction": "",
        "symbol": parsed["symbol"],
        "name": parsed["name"],
        "currency": currency,
        "quantity": quantity,
        "unitPrice": unit_price,
        "commission": 0.0,
        "netCashAmount": net_cash,
        "category": category,
        "balance": balance,
        "source": "statement",
    }, issue


def map_canonical(row):
    transaction_date = parse_date(_pick(row, "transaction_date", "date", "trade_date", "activity_date"))
    if not transaction_date:
        return None
    activity_type = _pick(row, "activity_type", "type")
    sub = _pick(row, "activity_sub_type", "activity_subtype", "sub_type", "subtype")
    return {
        "id": str(uuid.uuid4()),
        "occurredAt": transaction_date,
        "transactionDate": transaction_date,
        "settlementDate": parse_date(_pick(row, "settlement_date", "settle_date")) or transaction_date,
        "accountId": _pick(row, "account_id", "account"),
        "accountType": _pick(row, "account_type"),
        "activityType": activity_type or "Unknown",
        "activitySubType": sub,
        "description": _pick(row, "description", "memo", "details"),
        "direction": _pick(row, "direction").upper(),
        "symbol": _pick(row, "symbol", "ticker"),
        "name": _pick(row, "name", "security_name", "instrument"),
        "currency": (_pick(row, "currency", "ccy") or "CAD").upper(),
        "quantity": parse_number(_pick(row, "quantity", "qty")),
        "unitPrice": parse_number(_pick(row, "unit_price", "price", "fill_price")),
        "commission": abs(parse_number(_pick(row, "commission", "fee", "fees"))),
        "netCashAmount": parse_number(_pick(row, "net_cash_amount", "amount", "net_amount", "net_cash")),
        "category": categorize(activity_type, sub),
        "source": "canonical",
    }


def map_legacy(row):
    transaction_date = parse_date(_pick(row, "date", "transaction_date"))
    if not transaction_date:
        return None
    action = _pick(row, "action", "type", "activity").lower()
    activity_type, sub = "Other", ""
    if action in ("buy", "sell"):
        activity_type, sub = "Trade", action.upper()
    elif "dividend" in action:
        activity_type, sub = "Dividend", "DIVIDEND"
    elif "deposit" in action:
        activity_type, sub = "Deposit", "DEPOSIT"
    elif "withdraw" in action:
        activity_type, sub = "Withdrawal", "WITHDRAWAL"
    elif "interest" in action:
        activity_type, sub = "Interest", "INTEREST"
    elif "fee" in action:
        activity_type, sub = "Fee", "FEE"
    elif "fx" in action:
        activity_type, sub = "FxExchange", action.upper()
    elif action:
        activity_type, sub = action[:1].upper() + action[1:], action.upper()
    quantity = parse_number(_pick(row, "quantity", "qty"))
    if sub == "SELL":
        quantity = -abs(quantity)
    elif sub == "BUY":
        quantity = abs(quantity)
    return {
        "id": str(uuid.uuid4()),
        "occurredAt": transaction_date,
        "transactionDate": transaction_date,
        "settlementDate": transaction_date,
        "accountId": _pick(row, "account_id", "account") or "legacy",
        "accountType": _pick(row, "account_type"),
        "activityType": activity_type,
        "activitySubType": sub,
        "description": _pick(row, "description", "memo"),
        "direction": "",
        "symbol": _pick(row, "symbol", "ticker"),
        "name": _pick(row, "name", "security_name"),
        "currency": (_pick(row, "currency", "ccy") or "CAD").upper(),
        "quantity": quantity,
        "unitPrice": parse_number(_pick(row, "price", "unit_price")),
        "commission": abs(parse_number(_pick(row, "commission", "fee", "fees"))),
        "netCashAmount": parse_number(_pick(row, "amount", "net_cash_amount", "net_amount")),
        "category": categorize(activity_type, sub),
        "source": "legacy",
    }


def parse_csv(text, name=""):
    """-> {format, activities, skipped, footerStripped, countsByType, rowCount}"""
    text = _s(text).replace("﻿", "")
    footer = False
    lines = []
    for line in text.splitlines():
        if is_footer_line(line):
            footer = True
            continue
        lines.append(line)
    table = [row for row in csv.reader(io.StringIO("\n".join(lines)))]
    while table and all(not _s(c).strip() for c in table[-1]):
        table.pop()
    empty = {"format": "unknown", "activities": [], "skipped": [], "footerStripped": footer, "countsByType": {}, "rowCount": 0}
    if not table:
        empty["skipped"] = [{"row": 1, "message": "Empty file", "raw": ""}]
        return empty
    headers = [_s(h).replace("﻿", "").strip() for h in table[0]]
    fmt = detect_format(headers)
    norms = [normalize_header(h) for h in headers]
    skipped = []
    activities = []
    counts = {}
    if fmt == "unknown":
        skipped.append({"row": 1, "message": "Unrecognized CSV format. Expected a Wealthsimple activities export, a statement export with date/transaction/description/amount columns, or a Date/Action/Symbol file.", "raw": ",".join(headers)})
        return {"format": fmt, "activities": [], "skipped": skipped, "footerStripped": footer, "countsByType": {}, "rowCount": len(table) - 1}
    book = book_id_from_file_name(name)
    for i, cells in enumerate(table[1:], start=2):
        row = {norms[j]: (_s(cells[j]) if j < len(cells) else "") for j in range(len(norms))}
        if all(not v.strip() for v in row.values()):
            continue
        if is_footer_line(" ".join(row.values())):
            footer = True
            continue
        issue = None
        try:
            if fmt == "statement":
                activity, issue = map_statement(row, book)
            elif fmt == "legacy":
                activity = map_legacy(row)
            else:
                activity = map_canonical(row)
        except Exception as e:  # a bad row must not sink the file
            skipped.append({"row": i, "message": str(e) or "parse error", "raw": json.dumps(row)})
            continue
        if issue:
            skipped.append({"row": i, "message": issue, "raw": json.dumps(row)})
        if not activity:
            skipped.append({"row": i, "message": "Unparsed row (missing or invalid date)", "raw": json.dumps(row)})
            continue
        activities.append(activity)
        key = activity.get("activityType") or activity.get("category")
        counts[key] = counts.get(key, 0) + 1
    return {"format": fmt, "activities": activities, "skipped": skipped, "footerStripped": footer, "countsByType": counts, "rowCount": len(table) - 1}


def import_text(name, text):
    """Parse one CSV and merge it into the store. Returns the report."""
    report = parse_csv(text, name)
    rows = report["activities"]
    merged = store.merge_local_rows(rows) if rows else {"added": 0, "duplicates": 0}
    return {
        "ok": True,
        "file": _s(name).split("/")[-1],
        "format": report["format"],
        "rows": report["rowCount"],
        "added": merged.get("added", 0),
        "duplicates": merged.get("duplicates", 0),
        "skipped": report["skipped"][:20],
        "skippedCount": len(report["skipped"]),
        "footerStripped": report["footerStripped"],
        "countsByType": report["countsByType"],
    }


# --------------------------------------------------------------------------
# folder watching (the server scans; no browser needed)
# --------------------------------------------------------------------------


def is_junk_name(name):
    n = _s(name)
    return n.startswith("._") or "__MACOSX" in n.upper()


def is_csv_name(name):
    return _s(name).lower().endswith(".csv")


def list_csv_files(folder):
    out = []
    try:
        entries = sorted(os.listdir(folder))
    except OSError:
        return out
    for n in entries:
        p = os.path.join(folder, n)
        if not os.path.isfile(p) or not is_csv_name(n) or is_junk_name(n):
            continue
        try:
            st = os.stat(p)
        except OSError:
            continue
        if st.st_size == 0:
            continue
        out.append({"path": p, "name": n, "size": st.st_size, "mtime": int(st.st_mtime)})
    return out


def watch_folder():
    return store.get_meta(WATCH_META)


def set_watch_folder(path):
    p = os.path.expanduser(_s(path).strip())
    if not p:
        return {"ok": False, "error": "Folder path required"}
    if not os.path.isdir(p):
        return {"ok": False, "error": "Not a folder: %s" % p}
    store.set_meta(WATCH_META, p)
    return {"ok": True, "path": p}


def clear_watch_folder():
    store.set_meta(WATCH_META, "")
    store.set_meta(WATCH_FILES_META, "")
    store.set_meta(WATCH_LAST_META, "")


def _seen_files():
    raw = store.get_meta(WATCH_FILES_META)
    try:
        data = json.loads(raw) if raw else {}
    except ValueError:
        data = {}
    return data if isinstance(data, dict) else {}


def scan_folder(folder=None, force=False):
    """Import every top-level CSV in the folder; unchanged files are skipped
    unless force. Returns {ok, path, added, duplicates, files[], scannedAt}."""
    path = os.path.expanduser(_s(folder or watch_folder()).strip())
    if not path:
        return {"ok": False, "error": "No folder is being watched"}
    if not os.path.isdir(path):
        return {"ok": False, "error": "Folder not found: %s" % path, "path": path}
    seen = _seen_files()
    files = []
    added = 0
    duplicates = 0
    for f in list_csv_files(path):
        prev = seen.get(f["path"]) or {}
        if not force and prev.get("size") == f["size"] and prev.get("mtime") == f["mtime"]:
            files.append({"file": f["name"], "unchanged": True, "added": prev.get("added", 0), "duplicates": prev.get("duplicates", 0), "format": prev.get("format", "")})
            continue
        try:
            with open(f["path"], "r", encoding="utf-8-sig", errors="replace") as fh:
                text = fh.read()
        except OSError as e:
            files.append({"file": f["name"], "error": str(e)})
            continue
        rep = import_text(f["name"], text)
        added += rep["added"]
        duplicates += rep["duplicates"]
        seen[f["path"]] = {"size": f["size"], "mtime": f["mtime"], "added": rep["added"], "duplicates": rep["duplicates"], "format": rep["format"], "scannedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")}
        files.append({"file": f["name"], "unchanged": False, "added": rep["added"], "duplicates": rep["duplicates"], "format": rep["format"], "rows": rep["rows"], "skippedCount": rep["skippedCount"]})
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    store.set_meta(WATCH_FILES_META, json.dumps({k: v for k, v in seen.items() if os.path.exists(k)}))
    store.set_meta(WATCH_LAST_META, now)
    return {"ok": True, "path": path, "added": added, "duplicates": duplicates, "files": files, "scannedAt": now}


def status():
    path = watch_folder()
    seen = _seen_files()
    return {
        "ok": True,
        "path": path,
        "watching": bool(path),
        "lastScan": store.get_meta(WATCH_LAST_META),
        "files": [{"file": os.path.basename(k), "added": v.get("added", 0), "duplicates": v.get("duplicates", 0), "format": v.get("format", ""), "scannedAt": v.get("scannedAt", "")} for k, v in sorted(seen.items())],
    }
