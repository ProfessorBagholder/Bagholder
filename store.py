"""Local SQLite store for Bagholder activities, accounts, balances, and NAV.

The live store is ~/.bagholder/bagholder.db (or BAGHOLDER_HOME/bagholder.db).
"""

from __future__ import annotations

import json
import os
import sqlite3
import threading
import uuid
from datetime import datetime, timezone
from zoneinfo import ZoneInfo
from pathlib import Path

SCHEMA_VERSION = 3
ACTIVITY_PULL_TZ = ZoneInfo("America/Edmonton")
ACTIVITY_PULL_WEEKDAYS = (0, 1, 2, 3, 4)
ACTIVITY_PULL_HOUR = 14
ACTIVITY_PULL_MINUTE = 0

_home = None
_lock = threading.RLock()

_INVENTED_ACCOUNTS = frozenset(
    ("", "manual", "legacy", "statement", "canonical", "cad", "usd")
)


def set_home(path):
    global _home
    _home = Path(path) if path else None


def home():
    if _home is not None:
        return Path(_home)
    env = (os.environ.get("BAGHOLDER_HOME") or "").strip()
    if env:
        return Path(env)
    return Path.home() / ".bagholder"


def db_path():
    return home() / "bagholder.db"


def _now_iso():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _ensure_home():
    path = home()
    path.mkdir(mode=0o700, exist_ok=True)
    try:
        os.chmod(path, 0o700)
    except OSError:
        pass
    return path


def _connect():
    _ensure_home()
    path = db_path()
    conn = sqlite3.connect(str(path), check_same_thread=False)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    conn.execute("PRAGMA journal_mode=WAL")
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    return conn


def _init_schema(conn):
    conn.executescript(
        """
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS activities (
            id TEXT PRIMARY KEY,
            canonical_id TEXT,
            occurred_at TEXT,
            transaction_date TEXT NOT NULL,
            settlement_date TEXT,
            account_id TEXT,
            book_id TEXT,
            fifo_id TEXT,
            account_type TEXT,
            activity_type TEXT,
            activity_sub_type TEXT,
            description TEXT,
            direction TEXT,
            symbol TEXT,
            name TEXT,
            currency TEXT,
            quantity REAL,
            unit_price REAL,
            commission REAL,
            net_cash_amount REAL,
            category TEXT,
            balance REAL,
            source TEXT,
            raw_type TEXT,
            aft_type TEXT,
            counter_symbol TEXT,
            security_id TEXT
        );

        CREATE TABLE IF NOT EXISTS securities (
            id TEXT PRIMARY KEY,
            symbol TEXT,
            name TEXT,
            primary_exchange TEXT,
            primary_mic TEXT,
            currency TEXT,
            underlying_id TEXT,
            fetched_at TEXT
        );

        CREATE UNIQUE INDEX IF NOT EXISTS activities_canonical_id_uq
            ON activities (canonical_id)
            WHERE canonical_id IS NOT NULL AND canonical_id != '';

        CREATE TABLE IF NOT EXISTS accounts (
            id TEXT PRIMARY KEY,
            nickname TEXT,
            unified_account_type TEXT,
            currency TEXT,
            status TEXT,
            type TEXT,
            net_liquidation_value REAL
        );

        CREATE TABLE IF NOT EXISTS balances (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id TEXT,
            custodian_account_id TEXT,
            security_id TEXT,
            quantity REAL
        );

        CREATE TABLE IF NOT EXISTS nav_history (
            account_id TEXT NOT NULL DEFAULT '',
            date TEXT NOT NULL,
            equity REAL,
            currency TEXT,
            net_deposits REAL,
            PRIMARY KEY (account_id, date)
        );

        CREATE TABLE IF NOT EXISTS grouped_trades (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL
        );
        """
    )
    _migrate_nav_history(conn)
    _ensure_activity_security_id(conn)
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?, ?) "
        "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        ("schema_version", str(SCHEMA_VERSION)),
    )
    conn.commit()


def _migrate_nav_history(conn):
    """Rebuild nav_history with PRIMARY KEY (account_id, date). Old rows get account_id ''."""
    row = conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='nav_history'"
    ).fetchone()
    if not row:
        return
    info = conn.execute("PRAGMA table_info(nav_history)").fetchall()
    cols = {r["name"]: r for r in info}
    pk_cols = {r["name"] for r in info if r["pk"]}
    if "account_id" in cols and pk_cols == {"account_id", "date"}:
        return
    conn.execute(
        """
        CREATE TABLE nav_history_new (
            account_id TEXT NOT NULL DEFAULT '',
            date TEXT NOT NULL,
            equity REAL,
            currency TEXT,
            net_deposits REAL,
            PRIMARY KEY (account_id, date)
        )
        """
    )
    if "account_id" in cols:
        conn.execute(
            "INSERT INTO nav_history_new "
            "(account_id, date, equity, currency, net_deposits) "
            "SELECT COALESCE(account_id, ''), date, equity, currency, net_deposits "
            "FROM nav_history"
        )
    else:
        conn.execute(
            "INSERT INTO nav_history_new "
            "(account_id, date, equity, currency, net_deposits) "
            "SELECT '', date, equity, currency, net_deposits FROM nav_history"
        )
    conn.execute("DROP TABLE nav_history")
    conn.execute("ALTER TABLE nav_history_new RENAME TO nav_history")


def _ensure_activity_security_id(conn):
    cols = {r["name"] for r in conn.execute("PRAGMA table_info(activities)").fetchall()}
    if "security_id" not in cols:
        conn.execute("ALTER TABLE activities ADD COLUMN security_id TEXT")


def _relabel_option_trades(conn):
    """OPTIONS_BUY / OPTIONS_SELL were stored as LIMIT_ORDER / other. Treat as trades."""
    conn.execute(
        "UPDATE activities SET "
        "activity_sub_type = 'BUYTOOPEN', "
        "category = 'trade', "
        "quantity = ABS(quantity), "
        "net_cash_amount = -ABS(net_cash_amount) "
        "WHERE UPPER(REPLACE(IFNULL(raw_type,''), '-', '_')) = 'OPTIONS_BUY' "
        "AND UPPER(REPLACE(IFNULL(activity_sub_type,''), '-', '_')) "
        "NOT IN ('BUY', 'BUYTOOPEN', 'BTO', 'BUYTOCLOSE', 'BTC')"
    )
    conn.execute(
        "UPDATE activities SET "
        "activity_sub_type = 'SELLTOOPEN', "
        "category = 'trade', "
        "quantity = -ABS(quantity), "
        "net_cash_amount = ABS(net_cash_amount) "
        "WHERE UPPER(REPLACE(IFNULL(raw_type,''), '-', '_')) = 'OPTIONS_SELL' "
        "AND UPPER(REPLACE(IFNULL(activity_sub_type,''), '-', '_')) "
        "NOT IN ('SELL', 'SELLTOOPEN', 'STO', 'SELLTOCLOSE', 'STC', 'COVER')"
    )


def _is_option_symbol(symbol):
    compact = _s(symbol).strip().upper()
    if not compact:
        return False
    padded = " " + compact + " "
    if " CALL " in padded or " PUT " in padded:
        return True
    return compact.endswith(" C") or compact.endswith(" P")


def _cash_near(a, b, rel=0.02, abs_tol=0.02):
    return abs(a - b) <= max(abs_tol, rel * max(abs(a), abs(b), 1e-9))


def _scale_option_unit_prices(conn):
    """Fix Sync rows where option unit_price is 100× high (cash ≈ price × qty).

    Wealthsimple option `amount` is contract cash. Correct per-share price is
    amount / (qty × 100). The old `unit_price > 20` heuristic left cheap
    contracts 100× high. Sync never replaces existing rows, so repair on open.
    Already-correct rows (cash ≈ price × qty × 100) are left alone.
    """
    rows = conn.execute(
        "SELECT id, symbol, quantity, unit_price, net_cash_amount FROM activities"
    ).fetchall()
    for row in rows:
        if not _is_option_symbol(row["symbol"]):
            continue
        qty = abs(_num(row["quantity"], 0.0) or 0.0)
        px = abs(_num(row["unit_price"], 0.0) or 0.0)
        cash = abs(_num(row["net_cash_amount"], 0.0) or 0.0)
        if qty <= 0 or px <= 0 or cash <= 0:
            continue
        implied = px * qty
        if _cash_near(cash, implied * 100.0):
            continue
        if _cash_near(cash, implied):
            conn.execute(
                "UPDATE activities SET unit_price = ? WHERE id = ?",
                (px / 100.0, row["id"]),
            )


def ensure():
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            _relabel_option_trades(conn)
            _scale_option_unit_prices(conn)
            conn.commit()
        finally:
            conn.close()


def get_meta(key, default=""):
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            row = conn.execute(
                "SELECT value FROM meta WHERE key = ?", (key,)
            ).fetchone()
            if not row or row["value"] is None:
                return default
            return row["value"]
        finally:
            conn.close()


def set_meta(key, value):
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            conn.execute(
                "INSERT INTO meta(key, value) VALUES (?, ?) "
                "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (key, "" if value is None else str(value)),
            )
            conn.commit()
        finally:
            conn.close()


def activity_count():
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            row = conn.execute("SELECT COUNT(*) AS n FROM activities").fetchone()
            return int(row["n"] if row else 0)
        finally:
            conn.close()


def canonical_ids():
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            rows = conn.execute(
                "SELECT canonical_id FROM activities "
                "WHERE canonical_id IS NOT NULL AND canonical_id != ''"
            ).fetchall()
            return {r["canonical_id"] for r in rows}
        finally:
            conn.close()


def newest_ws_occurred_at():
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            row = conn.execute(
                "SELECT occurred_at, transaction_date FROM activities "
                "WHERE source = 'wealthsimple' "
                "ORDER BY COALESCE(occurred_at, transaction_date) DESC "
                "LIMIT 1"
            ).fetchone()
            if not row:
                row = conn.execute(
                    "SELECT occurred_at, transaction_date FROM activities "
                    "ORDER BY COALESCE(occurred_at, transaction_date) DESC "
                    "LIMIT 1"
                ).fetchone()
            if not row:
                return ""
            return (row["occurred_at"] or row["transaction_date"] or "").strip()
        finally:
            conn.close()


def incremental_start_date():
    """Date bound for a daily pull. Empty table must not call this for a full walk."""
    newest = newest_ws_occurred_at()
    if not newest:
        return ""
    day = newest.split("T", 1)[0][:10]
    return day


def _in_activity_pull_tz(dt):
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(ACTIVITY_PULL_TZ)


def activity_pull_due(now=None, interval_sec=None):
    """Due at 2:00 PM Mountain, Monday-Friday, after market close.

    interval_sec is ignored. One pull per weekday after 2:00 PM.
    """
    now = _in_activity_pull_tz(now or datetime.now(timezone.utc))
    if now.weekday() not in ACTIVITY_PULL_WEEKDAYS:
        return False
    close = now.replace(
        hour=ACTIVITY_PULL_HOUR,
        minute=ACTIVITY_PULL_MINUTE,
        second=0,
        microsecond=0,
    )
    if now < close:
        return False
    last = get_meta("last_activity_pull")
    if not last:
        return True
    try:
        if last.endswith("Z"):
            last = last[:-1] + "+00:00"
        then = datetime.fromisoformat(last)
        then = _in_activity_pull_tz(then)
    except ValueError:
        return True
    return then < close


def mark_activity_pulled(when=None):
    set_meta("last_activity_pull", when or _now_iso())


def looks_like_homemade_id(aid):
    s = str(aid or "").strip()
    if not s:
        return True
    if "|" in s:
        return True
    if s.lower().startswith("manual"):
        return True
    return False


def is_real_account(account_id):
    s = str(account_id or "").strip()
    if not s:
        return False
    if s.startswith("~"):
        return False
    if s.lower() in _INVENTED_ACCOUNTS:
        return False
    return True


def _s(v):
    if v is None:
        return ""
    return str(v)


def _num(v, default=None):
    if v is None or v == "":
        return default
    try:
        return float(v)
    except (TypeError, ValueError):
        return default


def _round_qty(v):
    n = _num(v, 0.0)
    return round(float(n or 0.0), 8)


def trade_side(act):
    sub = _s(act.get("activitySubType") or act.get("activity_sub_type")).upper()
    compact = sub.replace(" ", "").replace("_", "").replace("-", "")
    if compact in (
        "BUY",
        "BUYTOOPEN",
        "BTO",
        "BUYTOCLOSE",
        "BTC",
    ) or compact.startswith("BUY"):
        return "BUY"
    if compact in (
        "SELL",
        "SELLTOOPEN",
        "STO",
        "SELLTOCLOSE",
        "STC",
    ) or compact.startswith("SELL"):
        return "SELL"
    typ = _s(act.get("activityType") or act.get("activity_type")).upper()
    tcompact = typ.replace(" ", "").replace("_", "").replace("-", "")
    if tcompact.startswith("BUY"):
        return "BUY"
    if tcompact.startswith("SELL"):
        return "SELL"
    qty = _num(act.get("quantity"), 0.0) or 0.0
    if qty > 0:
        return "BUY"
    if qty < 0:
        return "SELL"
    return ""


def field_match_key(act, include_account=True):
    date = _s(act.get("transactionDate") or act.get("transaction_date") or "")[:10]
    if not date:
        occurred = _s(act.get("occurredAt") or act.get("occurred_at"))
        date = occurred.split("T", 1)[0][:10]
    account = ""
    if include_account:
        aid = _s(act.get("accountId") or act.get("account_id"))
        if is_real_account(aid):
            account = aid
    return (
        date,
        account,
        _s(act.get("symbol")).strip().upper(),
        _round_qty(act.get("quantity")),
        _round_qty(act.get("unitPrice") if "unitPrice" in act or "unit_price" in act else act.get("unit_price")),
        _round_qty(act.get("netCashAmount") if "netCashAmount" in act or "net_cash_amount" in act else act.get("net_cash_amount")),
    )


def link_match_key(act, include_account=True):
    date = _s(act.get("transactionDate") or act.get("transaction_date") or "")[:10]
    if not date:
        occurred = _s(act.get("occurredAt") or act.get("occurred_at"))
        date = occurred.split("T", 1)[0][:10]
    account = ""
    if include_account:
        aid = _s(act.get("accountId") or act.get("account_id"))
        if is_real_account(aid):
            account = aid
    return (
        _s(act.get("symbol")).strip().upper(),
        trade_side(act),
        _round_qty(act.get("quantity")),
        _round_qty(act.get("unitPrice") if "unitPrice" in act or "unit_price" in act else act.get("unit_price")),
        date,
        account,
    )


def _new_id():
    return str(uuid.uuid4())


def _canonical_from_row(act, source):
    if source != "wealthsimple":
        return None
    cid = _s(act.get("canonicalId") or act.get("canonical_id")).strip()
    if cid and not looks_like_homemade_id(cid):
        return cid
    old_id = _s(act.get("id")).strip()
    if old_id and not looks_like_homemade_id(old_id):
        return old_id
    return None


def _row_to_activity(row):
    cid = row["canonical_id"] or None
    return {
        "id": row["id"],
        "canonicalId": cid,
        "occurredAt": row["occurred_at"] or "",
        "transactionDate": row["transaction_date"] or "",
        "settlementDate": row["settlement_date"] or row["transaction_date"] or "",
        "accountId": row["account_id"] or "",
        "bookId": row["book_id"] or row["account_id"] or "",
        "fifoId": row["fifo_id"] or row["account_id"] or "",
        "accountType": row["account_type"] or "",
        "activityType": row["activity_type"] or "",
        "activitySubType": row["activity_sub_type"] or "",
        "description": row["description"] or "",
        "direction": row["direction"] or "",
        "symbol": row["symbol"] or "",
        "name": row["name"] or "",
        "currency": row["currency"] or "",
        "quantity": row["quantity"],
        "unitPrice": row["unit_price"],
        "commission": row["commission"],
        "netCashAmount": row["net_cash_amount"],
        "category": row["category"] or "",
        "balance": row["balance"],
        "source": row["source"] or "",
        "rawType": row["raw_type"] or "",
        "aftType": row["aft_type"] or "",
        "counterSymbol": row["counter_symbol"] or "",
        "securityId": row["security_id"] or None,
    }


def _insert_params(act, assigned_id, canonical_id):
    occurred = _s(act.get("occurredAt") or act.get("occurred_at")).strip()
    date = _s(act.get("transactionDate") or act.get("transaction_date")).strip()
    if not date and occurred:
        date = occurred.split("T", 1)[0][:10]
    if occurred and "T" not in occurred:
        # date-only source (typed / CSV) stays date-only
        occurred = occurred[:10]
    settle = _s(act.get("settlementDate") or act.get("settlement_date")).strip() or date
    account_id = _s(act.get("accountId") or act.get("account_id"))
    return (
        assigned_id,
        canonical_id,
        occurred,
        date,
        settle,
        account_id,
        _s(act.get("bookId") or act.get("book_id") or account_id),
        _s(act.get("fifoId") or act.get("fifo_id") or account_id),
        _s(act.get("accountType") or act.get("account_type")),
        _s(act.get("activityType") or act.get("activity_type")),
        _s(act.get("activitySubType") or act.get("activity_sub_type")),
        _s(act.get("description")),
        _s(act.get("direction")),
        _s(act.get("symbol")),
        _s(act.get("name")),
        _s(act.get("currency")),
        _num(act.get("quantity"), 0.0),
        _num(act.get("unitPrice") if "unitPrice" in act else act.get("unit_price"), 0.0),
        _num(act.get("commission"), 0.0),
        _num(act.get("netCashAmount") if "netCashAmount" in act else act.get("net_cash_amount"), 0.0),
        _s(act.get("category")),
        _num(act.get("balance"), None),
        _s(act.get("source")),
        _s(act.get("rawType") or act.get("raw_type")),
        _s(act.get("aftType") or act.get("aft_type")),
        _s(act.get("counterSymbol") or act.get("counter_symbol")),
        _s(act.get("securityId") or act.get("security_id")).strip() or None,
    )


_INSERT_SQL = """
    INSERT INTO activities (
        id, canonical_id, occurred_at, transaction_date, settlement_date,
        account_id, book_id, fifo_id, account_type, activity_type,
        activity_sub_type, description, direction, symbol, name, currency,
        quantity, unit_price, commission, net_cash_amount, category, balance,
        source, raw_type, aft_type, counter_symbol, security_id
    ) VALUES (
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
    )
"""


def insert_activity(act, canonical_id=None, assigned_id=None):
    """Insert one row. Caller decides canonical_id. Never fabricates one."""
    source = _s(act.get("source")) or "wealthsimple"
    if canonical_id is None and source == "wealthsimple":
        canonical_id = _canonical_from_row(act, source)
    if source != "wealthsimple":
        canonical_id = None
    if canonical_id == "":
        canonical_id = None
    aid = assigned_id or _s(act.get("id")).strip()
    if not aid or looks_like_homemade_id(aid):
        aid = _new_id()
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            conn.execute(_INSERT_SQL, _insert_params(act, aid, canonical_id))
            conn.commit()
            row = conn.execute(
                "SELECT * FROM activities WHERE id = ?", (aid,)
            ).fetchone()
            return _row_to_activity(row)
        finally:
            conn.close()


def insert_local(act):
    """Typed-in or CSV row: Bagholder id, never a fabricated canonicalId."""
    payload = dict(act or {})
    source = _s(payload.get("source")) or "manual"
    if source == "wealthsimple":
        source = "manual"
    payload["source"] = source
    payload.pop("canonicalId", None)
    payload.pop("canonical_id", None)
    return insert_activity(payload, canonical_id=None)


def _all_activities(conn):
    rows = conn.execute(
        "SELECT * FROM activities ORDER BY "
        "COALESCE(occurred_at, transaction_date) ASC, id ASC"
    ).fetchall()
    return [_row_to_activity(r) for r in rows]


def find_link_candidates(act):
    """Unlinked local rows that match symbol, side, qty, price, date (account if real)."""
    include_account = is_real_account(
        act.get("accountId") or act.get("account_id")
    )
    target = link_match_key(act, include_account=include_account)
    matches = []
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            rows = conn.execute(
                "SELECT * FROM activities "
                "WHERE canonical_id IS NULL OR canonical_id = ''"
            ).fetchall()
            for row in rows:
                mapped = _row_to_activity(row)
                if link_match_key(mapped, include_account=include_account) == target:
                    matches.append(mapped)
        finally:
            conn.close()
    return matches


def stamp_canonical_id(activity_id, canonical_id):
    cid = _s(canonical_id).strip()
    if not cid or looks_like_homemade_id(cid):
        return False
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            conn.execute(
                "UPDATE activities SET canonical_id = ? WHERE id = ? "
                "AND (canonical_id IS NULL OR canonical_id = '')",
                (cid, activity_id),
            )
            conn.commit()
            return conn.total_changes > 0
        finally:
            conn.close()


def apply_wealthsimple_mapped(rows):
    """Insert if canonicalId is new. Never replace an existing row.

    Linking a typed/CSV row: exactly one field match stamps canonicalId.
    None → insert. Several → do not guess; insert the Wealthsimple row.
    """
    inserted = 0
    linked = 0
    skipped = 0
    known = canonical_ids()
    for raw in rows or []:
        if not raw:
            continue
        row = dict(raw)
        row["source"] = "wealthsimple"
        cid = _s(row.get("canonicalId") or row.get("canonical_id")).strip()
        if not cid or looks_like_homemade_id(cid):
            continue
        if cid in known:
            sid = _s(row.get("securityId") or row.get("security_id")).strip()
            if sid:
                with _lock:
                    conn = _connect()
                    try:
                        _init_schema(conn)
                        conn.execute(
                            "UPDATE activities SET security_id = ? "
                            "WHERE canonical_id = ? "
                            "AND (security_id IS NULL OR security_id = '')",
                            (sid, cid),
                        )
                        conn.commit()
                    finally:
                        conn.close()
            skipped += 1
            continue
        matches = find_link_candidates(row)
        if len(matches) == 1:
            if stamp_canonical_id(matches[0]["id"], cid):
                known.add(cid)
                linked += 1
                continue
        insert_activity(row, canonical_id=cid)
        known.add(cid)
        inserted += 1
    return {"inserted": inserted, "linked": linked, "skipped": skipped}


def merge_local_rows(rows):
    """CSV / typed merge on date, account, symbol, quantity, price, cash — not id."""
    stored = []
    added = 0
    duplicates = 0
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            existing_counts = {}
            for a in _all_activities(conn):
                k = field_match_key(a)
                existing_counts[k] = existing_counts.get(k, 0) + 1
        finally:
            conn.close()

    incoming_seen = {}
    for raw in rows or []:
        if not raw:
            continue
        row = dict(raw)
        source = _s(row.get("source")) or "csv"
        if source == "wealthsimple":
            cid = _canonical_from_row(row, "wealthsimple")
            if cid:
                result = apply_wealthsimple_mapped([row])
                added += result["inserted"] + result["linked"]
                if result["skipped"]:
                    duplicates += result["skipped"]
                continue
            source = "csv"
        row["source"] = source
        row.pop("canonicalId", None)
        row.pop("canonical_id", None)
        k = field_match_key(row)
        n = incoming_seen.get(k, 0) + 1
        incoming_seen[k] = n
        if n <= existing_counts.get(k, 0):
            duplicates += 1
            continue
        saved = insert_local(row)
        existing_counts[k] = existing_counts.get(k, 0) + 1
        stored.append(saved)
        added += 1
    return {"ok": True, "added": added, "duplicates": duplicates, "activities": stored}


def replace_accounts(accounts):
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            conn.execute("DELETE FROM accounts")
            for acc in accounts or []:
                if not isinstance(acc, dict):
                    continue
                aid = _s(acc.get("id"))
                if not aid:
                    continue
                nlv = _num(acc.get("netLiquidationValue"), None)
                conn.execute(
                    "INSERT INTO accounts ("
                    "id, nickname, unified_account_type, currency, status, type, "
                    "net_liquidation_value) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    (
                        aid,
                        _s(acc.get("nickname")),
                        _s(acc.get("unifiedAccountType") or acc.get("unified_account_type")),
                        _s(acc.get("currency")),
                        _s(acc.get("status")),
                        _s(acc.get("type")),
                        nlv,
                    ),
                )
            conn.commit()
        finally:
            conn.close()


def replace_balances(balances):
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            conn.execute("DELETE FROM balances")
            for b in balances or []:
                if not isinstance(b, dict):
                    continue
                conn.execute(
                    "INSERT INTO balances ("
                    "account_id, custodian_account_id, security_id, quantity"
                    ") VALUES (?, ?, ?, ?)",
                    (
                        _s(b.get("accountId") or b.get("account_id")),
                        _s(b.get("custodianAccountId") or b.get("custodian_account_id")),
                        _s(b.get("securityId") or b.get("security_id")),
                        _num(b.get("quantity"), None),
                    ),
                )
            conn.commit()
        finally:
            conn.close()


def _nav_point_from_row(r):
    rec = {
        "date": r["date"],
        "equity": r["equity"],
        "currency": r["currency"] or "CAD",
    }
    if r["net_deposits"] is not None:
        rec["netDeposits"] = r["net_deposits"]
    return rec


def nav_last_dates():
    """Newest stored daily-value date per account_id. Empty string is All."""
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            rows = conn.execute(
                "SELECT account_id, MAX(date) AS last FROM nav_history GROUP BY account_id"
            ).fetchall()
            out = {}
            for r in rows:
                last = r["last"]
                if not last:
                    continue
                out[r["account_id"] or ""] = last
            return out
        finally:
            conn.close()


def _write_nav_points(conn, points):
    for rec in points or []:
        if not isinstance(rec, dict):
            continue
        day = _s(rec.get("date"))[:10]
        if not day:
            continue
        equity = _num(rec.get("equity"), None)
        if equity is None:
            continue
        account_id = _s(rec.get("accountId") if rec.get("accountId") is not None else rec.get("account_id"))
        conn.execute(
            "INSERT INTO nav_history "
            "(account_id, date, equity, currency, net_deposits) "
            "VALUES (?, ?, ?, ?, ?) "
            "ON CONFLICT(account_id, date) DO UPDATE SET "
            "equity = excluded.equity, "
            "currency = excluded.currency, "
            "net_deposits = excluded.net_deposits",
            (
                account_id,
                day,
                equity,
                _s(rec.get("currency") or "CAD"),
                _num(rec.get("netDeposits") if rec.get("netDeposits") is not None else rec.get("net_deposits"), None),
            ),
        )


def upsert_nav(points):
    """Insert or update daily-value rows. Does not delete existing days."""
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            _write_nav_points(conn, points)
            conn.commit()
        finally:
            conn.close()


def replace_nav(points):
    """Replace the whole nav_history table from identity-wide + per-nickname series."""
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            conn.execute("DELETE FROM nav_history")
            _write_nav_points(conn, points)
            conn.commit()
        finally:
            conn.close()



def _clean_trade_groups(raw):
    if not isinstance(raw, list):
        return []
    out = []
    seen = set()
    for item in raw:
        if not isinstance(item, dict):
            continue
        gid = str(item.get("id") or "").strip()
        members = item.get("members")
        if not gid or gid in seen or not isinstance(members, list):
            continue
        keys = []
        used = set()
        for m in members:
            k = str(m or "").strip()
            if not k or k in used:
                continue
            used.add(k)
            keys.append(k)
        if not keys:
            continue
        seen.add(gid)
        out.append({"id": gid, "locked": bool(item.get("locked")), "members": keys})
    return out


def trade_groups():
    raw = get_meta("trade_groups")
    if not raw:
        return []
    try:
        data = json.loads(raw)
    except ValueError:
        return []
    return _clean_trade_groups(data)


def save_trade_groups(groups):
    clean = _clean_trade_groups(groups)
    set_meta("trade_groups", json.dumps(clean))
    return clean


def _clean_trade_notes(raw):
    if not isinstance(raw, dict):
        return {}
    out = {}
    for key, val in raw.items():
        kid = str(key or "").strip()
        if not kid or not isinstance(val, dict):
            continue
        thesis = str(val.get("thesis") or "")
        tag = str(val.get("tag") or "")
        grade = str(val.get("grade") or "")
        if grade not in ("A", "B", "C", "F"):
            grade = ""
        if not thesis and not tag and not grade:
            continue
        out[kid] = {"thesis": thesis, "tag": tag, "grade": grade, "tradeId": kid}
    return out


def trade_notes():
    raw = get_meta("trade_notes")
    if not raw:
        return {}
    try:
        data = json.loads(raw)
    except ValueError:
        return {}
    return _clean_trade_notes(data)


def save_trade_notes(notes):
    clean = _clean_trade_notes(notes if isinstance(notes, dict) else {})
    set_meta("trade_notes", json.dumps(clean))
    return clean


def _security_from_row(r):
    return {
        "id": r["id"],
        "symbol": r["symbol"] or "",
        "name": r["name"] or "",
        "primaryExchange": r["primary_exchange"] or "",
        "primaryMic": r["primary_mic"] or "",
        "currency": r["currency"] or "",
        "underlyingId": r["underlying_id"] or None,
    }


def upsert_securities(rows):
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            now = _now_iso()
            for raw in rows or []:
                if not isinstance(raw, dict):
                    continue
                sid = _s(raw.get("id")).strip()
                if not sid:
                    continue
                under = _s(
                    raw.get("underlyingId")
                    if raw.get("underlyingId") is not None
                    else raw.get("underlying_id")
                ).strip()
                conn.execute(
                    "INSERT INTO securities ("
                    "id, symbol, name, primary_exchange, primary_mic, "
                    "currency, underlying_id, fetched_at"
                    ") VALUES (?, ?, ?, ?, ?, ?, ?, ?) "
                    "ON CONFLICT(id) DO UPDATE SET "
                    "symbol = excluded.symbol, "
                    "name = excluded.name, "
                    "primary_exchange = excluded.primary_exchange, "
                    "primary_mic = excluded.primary_mic, "
                    "currency = excluded.currency, "
                    "underlying_id = excluded.underlying_id, "
                    "fetched_at = excluded.fetched_at",
                    (
                        sid,
                        _s(raw.get("symbol")),
                        _s(raw.get("name")),
                        _s(raw.get("primaryExchange") or raw.get("primary_exchange")),
                        _s(raw.get("primaryMic") or raw.get("primary_mic")),
                        _s(raw.get("currency")),
                        under or None,
                        _s(raw.get("fetchedAt") or raw.get("fetched_at")) or now,
                    ),
                )
            conn.commit()
        finally:
            conn.close()


def list_securities():
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            rows = conn.execute("SELECT * FROM securities ORDER BY id").fetchall()
            return [_security_from_row(r) for r in rows]
        finally:
            conn.close()


def missing_security_ids(ids):
    wanted = []
    seen = set()
    for raw in ids or []:
        sid = _s(raw).strip()
        if not sid or sid in seen:
            continue
        seen.add(sid)
        wanted.append(sid)
    if not wanted:
        return []
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            have = set()
            for i in range(0, len(wanted), 400):
                chunk = wanted[i : i + 400]
                qmarks = ",".join("?" * len(chunk))
                for r in conn.execute(
                    "SELECT id FROM securities WHERE id IN (%s)" % qmarks, chunk
                ).fetchall():
                    have.add(r["id"])
            return [sid for sid in wanted if sid not in have]
        finally:
            conn.close()


def needs_security_id_backfill():
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            row = conn.execute(
                "SELECT 1 AS n FROM activities "
                "WHERE source = 'wealthsimple' "
                "AND IFNULL(symbol, '') != '' "
                "AND (security_id IS NULL OR security_id = '') "
                "LIMIT 1"
            ).fetchone()
            return bool(row)
        finally:
            conn.close()


def snapshot():
    ensure()
    with _lock:
        conn = _connect()
        try:
            _init_schema(conn)
            activities = _all_activities(conn)
            accounts = []
            for r in conn.execute("SELECT * FROM accounts ORDER BY id").fetchall():
                accounts.append(
                    {
                        "id": r["id"],
                        "nickname": r["nickname"] or "",
                        "unifiedAccountType": r["unified_account_type"] or "",
                        "currency": r["currency"] or "",
                        "status": r["status"] or "",
                        "type": r["type"] or "",
                        "netLiquidationValue": r["net_liquidation_value"],
                    }
                )
            balances = []
            for r in conn.execute("SELECT * FROM balances").fetchall():
                balances.append(
                    {
                        "accountId": r["account_id"],
                        "custodianAccountId": r["custodian_account_id"],
                        "securityId": r["security_id"],
                        "quantity": r["quantity"],
                    }
                )
            nav = []
            nav_by_account = {}
            for r in conn.execute(
                "SELECT * FROM nav_history ORDER BY account_id, date"
            ).fetchall():
                rec = _nav_point_from_row(r)
                aid = r["account_id"] or ""
                if not aid:
                    nav.append(rec)
                else:
                    nav_by_account.setdefault(aid, []).append(rec)
            synced = get_meta("synced_at")
            groups_raw = get_meta("trade_groups")
            try:
                groups = _clean_trade_groups(json.loads(groups_raw) if groups_raw else [])
            except ValueError:
                groups = []
            notes_raw = get_meta("trade_notes")
            try:
                notes = _clean_trade_notes(json.loads(notes_raw) if notes_raw else {})
            except ValueError:
                notes = {}
            securities = [
                _security_from_row(r)
                for r in conn.execute("SELECT * FROM securities ORDER BY id").fetchall()
            ]
            return {
                "activities": activities,
                "accounts": accounts,
                "balances": balances,
                "navHistory": nav,
                "navByAccount": nav_by_account,
                "syncedAt": synced,
                "tradeGroups": groups,
                "notes": notes,
                "securities": securities,
            }
        finally:
            conn.close()
