"""Local SQLite store for Bagholder activities, accounts, balances, and NAV.

The live store is ~/.bagholder/bagholder.db (or BAGHOLDER_HOME/bagholder.db).
"""

from __future__ import annotations

import json
import os
import sqlite3
import threading
import uuid
from datetime import timedelta, datetime, timezone
from zoneinfo import ZoneInfo
from pathlib import Path

SCHEMA_VERSION = 4
FX_PAIR = "USDCAD"
BENCHMARK_SYMBOL = "SP500"
JOURNAL_META = "journal_v2"
OPTION_UNIT_PRICE_SCALE_META = "option_unit_price_scale_v1"
OPTION_RELABEL_META = "option_relabel_rows_v1"
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


# Connections are pooled. Opening one and running the schema script again for
# every read cost about a millisecond here and ten times that on a small
# machine, and a model request makes eighteen of them. A borrowed connection
# goes back to the pool on close(), so no call site changes.
_pool = []
_pool_path = None
_pool_ready = set()
_POOL_MAX = 4


class _Borrowed:
    """A pooled connection. close() returns it to the pool instead of closing it."""

    __slots__ = ("_conn",)

    def __init__(self, conn):
        object.__setattr__(self, "_conn", conn)

    def __getattr__(self, name):
        return getattr(object.__getattribute__(self, "_conn"), name)

    def __setattr__(self, name, value):
        setattr(object.__getattribute__(self, "_conn"), name, value)

    def close(self):
        _release(self)


def _open_connection():
    path = db_path()
    conn = sqlite3.connect(str(path), check_same_thread=False)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    conn.execute("PRAGMA journal_mode=WAL")
    # WAL keeps the database consistent through a crash of the app or the
    # machine; NORMAL drops the fsync on every commit, which a card-backed
    # disk charges dearly for. Only a power cut can lose the last commits.
    conn.execute("PRAGMA synchronous=NORMAL")
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
    return conn


def _drop_pool():
    """Close every pooled connection: the database being read has changed."""
    global _pool, _pool_path
    for conn in _pool:
        try:
            conn.close()
        except sqlite3.Error:
            pass
    _pool = []
    _pool_path = None
    _pool_ready.clear()


def _connect():
    """A connection from the pool, or a new one. Give it back with close()."""
    global _pool_path
    _ensure_home()
    path = str(db_path())
    with _lock:
        if _pool_path != path:
            _drop_pool()
            _pool_path = path
        conn = _pool.pop() if _pool else _open_connection()
    return _Borrowed(conn)


def _release(borrowed):
    conn = object.__getattribute__(borrowed, "_conn")
    with _lock:
        if _pool_path == str(db_path()) and len(_pool) < _POOL_MAX:
            _pool.append(conn)
            return
    try:
        _pool_ready.discard(id(conn))
        conn.close()
    except sqlite3.Error:
        pass


def close_all():
    """Release every pooled connection. For tests and for a home that moves."""
    with _lock:
        _drop_pool()


def _conn_key(conn):
    return id(object.__getattribute__(conn, "_conn")) if isinstance(conn, _Borrowed) else id(conn)


def _ready(conn):
    """The schema, once per connection. Creating what is missing and running the
    migrations is the same work every time on a database that has already been
    through it, and a single model request asked for it eighteen times. The
    stamped schema version is still read every time, so a database that is
    replaced or rolled back under a live connection is migrated as before."""
    key = _conn_key(conn)
    if key in _pool_ready:
        try:
            row = conn.execute("SELECT value FROM meta WHERE key = 'schema_version'").fetchone()
        except sqlite3.Error:
            row = None
        if row is not None and str(row["value"]) == str(SCHEMA_VERSION):
            return
        _pool_ready.discard(key)
    _init_schema(conn)


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

        CREATE TABLE IF NOT EXISTS fx_rates (
            pair TEXT NOT NULL,
            date TEXT NOT NULL,
            rate REAL NOT NULL,
            PRIMARY KEY (pair, date)
        );

        CREATE TABLE IF NOT EXISTS benchmark_prices (
            symbol TEXT NOT NULL,
            date TEXT NOT NULL,
            close REAL NOT NULL,
            PRIMARY KEY (symbol, date)
        );

        CREATE TABLE IF NOT EXISTS distributions (
            symbol TEXT NOT NULL,
            ex_date TEXT NOT NULL,
            pay_date TEXT,
            amount REAL NOT NULL,
            currency TEXT,
            source TEXT NOT NULL DEFAULT 'tmx',
            PRIMARY KEY (symbol, ex_date, source)
        );

        CREATE TABLE IF NOT EXISTS quotes (
            symbol TEXT PRIMARY KEY,
            price REAL,
            dividend_amount REAL,
            dividend_frequency TEXT,
            ex_dividend_date TEXT,
            source TEXT,
            fetched_at TEXT
        );

        CREATE TABLE IF NOT EXISTS margin (
            account_id TEXT PRIMARY KEY,
            buying_power REAL,
            currency TEXT,
            unavailable TEXT,
            fetched_at TEXT
        );

        CREATE TABLE IF NOT EXISTS distribution_fetches (
            symbol TEXT PRIMARY KEY,
            fetched_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS price_history (
            symbol TEXT NOT NULL,
            date TEXT NOT NULL,
            open REAL,
            high REAL,
            low REAL,
            close REAL NOT NULL,
            volume REAL,
            source TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (symbol, date)
        );

        CREATE TABLE IF NOT EXISTS history_fetches (
            symbol TEXT PRIMARY KEY,
            start TEXT NOT NULL,
            fetched_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS price_bars (
            symbol TEXT NOT NULL,
            tf TEXT NOT NULL,
            ts INTEGER NOT NULL,
            open REAL,
            high REAL,
            low REAL,
            close REAL NOT NULL,
            volume REAL,
            source TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (symbol, tf, ts)
        );

        CREATE TABLE IF NOT EXISTS bar_fetches (
            symbol TEXT NOT NULL,
            tf TEXT NOT NULL,
            start_ts INTEGER NOT NULL,
            fetched_at TEXT NOT NULL,
            PRIMARY KEY (symbol, tf)
        );

        CREATE TABLE IF NOT EXISTS brackets (
            id TEXT PRIMARY KEY,
            order_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            account_id TEXT NOT NULL,
            security_id TEXT NOT NULL,
            symbol TEXT,
            currency TEXT,
            quantity REAL,
            tif TEXT,
            sl_kind TEXT,
            sl_price REAL,
            sl_trail REAL,
            sl_trail_unit TEXT,
            sl_order_id TEXT,
            sl_native INTEGER,
            sl_mode TEXT,
            high_water REAL,
            tp_price REAL,
            tp_order_id TEXT,
            status TEXT NOT NULL,
            outcome TEXT,
            error TEXT,
            attempts INTEGER,
            moved_at TEXT,
            armed_at TEXT,
            seen_held INTEGER,
            missed_at TEXT,
            updated_at TEXT
        );

        CREATE TABLE IF NOT EXISTS exposures (
            key TEXT PRIMARY KEY,
            sectors TEXT,
            countries TEXT,
            coverage REAL,
            source TEXT,
            as_of TEXT,
            industry TEXT,
            error TEXT,
            fetched_at TEXT
        );

        CREATE TABLE IF NOT EXISTS watchlist (
            symbol TEXT NOT NULL,
            exchange TEXT NOT NULL DEFAULT '',
            name TEXT,
            currency TEXT,
            security_id TEXT,
            added_at TEXT,
            PRIMARY KEY (symbol, exchange)
        );

        CREATE TABLE IF NOT EXISTS news (
            id TEXT NOT NULL,
            symbol TEXT NOT NULL,
            exchange TEXT NOT NULL DEFAULT '',
            source TEXT,
            headline TEXT,
            wire TEXT,
            url TEXT,
            published_at TEXT,
            fetched_at TEXT,
            PRIMARY KEY (id, symbol, exchange)
        );
        CREATE INDEX IF NOT EXISTS news_published ON news (published_at);

        CREATE TABLE IF NOT EXISTS universes (
            key TEXT NOT NULL,
            symbol TEXT NOT NULL,
            name TEXT,
            value REAL,
            percent_change REAL,
            sector TEXT,
            country TEXT,
            fetched_at TEXT,
            PRIMARY KEY (key, symbol)
        );

        CREATE TABLE IF NOT EXISTS orders (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            account_id TEXT NOT NULL,
            account TEXT,
            security_id TEXT NOT NULL,
            symbol TEXT,
            currency TEXT,
            side TEXT NOT NULL,
            type TEXT NOT NULL,
            quantity REAL NOT NULL,
            limit_price REAL,
            stop_price REAL,
            tif TEXT NOT NULL,
            stop_loss TEXT,
            take_profit TEXT,
            status TEXT NOT NULL,
            ws_order_id TEXT,
            error TEXT,
            request TEXT,
            updated_at TEXT
        );
        """
    )
    _migrate_nav_history(conn)
    _ensure_bar_columns(conn)
    _ensure_activity_security_id(conn)
    _migrate_spy_meta(conn)
    _ensure_quote_columns(conn)
    _ensure_order_columns(conn)
    _ensure_account_columns(conn)
    _migrate_history_sources(conn)
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?, ?) "
        "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        ("schema_version", str(SCHEMA_VERSION)),
    )
    conn.commit()
    _pool_ready.add(_conn_key(conn))


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


def _relabel_when_rows_changed(conn):
    """Relabel newly arrived option rows, and nothing else.

    Sync never replaces a stored row, so rows land with Wealthsimple's own
    labels and have to be relabelled after every pull. Doing it on every read
    instead meant six UPDATE statements over the whole table each time the page
    asked for anything. The fingerprint of the rows is stamped once they are
    relabelled; an unchanged table is left alone."""
    row = conn.execute(
        "SELECT COUNT(*) AS n, MAX(COALESCE(occurred_at, transaction_date)) AS m FROM activities"
    ).fetchone()
    key = "%s|%s" % (row["n"], row["m"])
    stamped = conn.execute("SELECT value FROM meta WHERE key = ?", (OPTION_RELABEL_META,)).fetchone()
    if stamped and stamped["value"] == key:
        return False
    _relabel_option_trades(conn)
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?, ?) "
        "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (OPTION_RELABEL_META, key),
    )
    return True


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
    _relabel_option_closes(conn)


def _relabel_option_closes(conn):
    """OPTIONS_MULTILEG / *EXPIR* / *ASSIGN* close and open semantics.

    Sync never replaces existing rows. Re-touch even if PR #22 already
    set category trade/option_event with the old close-only labels.
    """
    raw = "UPPER(REPLACE(IFNULL(raw_type,''), '-', '_'))"
    conn.execute(
        "UPDATE activities SET "
        "activity_type = 'OPTIONS_BUY', "
        "activity_sub_type = 'BUYTOCLOSE', "
        "category = 'trade' "
        f"WHERE {raw} LIKE '%MULTILEG%' "
        "AND IFNULL(net_cash_amount, 0) < 0"
    )
    conn.execute(
        "UPDATE activities SET "
        "activity_type = 'OPTIONS_SELL', "
        "activity_sub_type = 'SELLTOOPEN', "
        "category = 'trade' "
        f"WHERE {raw} LIKE '%MULTILEG%' "
        "AND IFNULL(net_cash_amount, 0) >= 0"
    )
    conn.execute(
        "UPDATE activities SET "
        "activity_type = 'ASSIGN', "
        "activity_sub_type = 'BUYTOCLOSE', "
        "category = 'option_event', "
        "quantity = ABS(quantity), "
        "unit_price = 0 "
        f"WHERE {raw} LIKE '%ASSIGN%'"
    )
    conn.execute(
        "UPDATE activities SET "
        "activity_type = 'EXPIR', "
        "activity_sub_type = 'BUY', "
        "category = 'option_event', "
        "quantity = ABS(quantity), "
        "unit_price = CASE WHEN ABS(IFNULL(net_cash_amount, 0)) < 1e-12 THEN 0 ELSE unit_price END "
        f"WHERE {raw} LIKE '%SHORT%EXPIR%'"
    )
    conn.execute(
        "UPDATE activities SET "
        "activity_type = 'EXPIR', "
        "activity_sub_type = 'SELL', "
        "category = 'option_event', "
        "quantity = -ABS(quantity), "
        "unit_price = CASE WHEN ABS(IFNULL(net_cash_amount, 0)) < 1e-12 THEN 0 ELSE unit_price END "
        f"WHERE {raw} LIKE '%EXPIR%' "
        f"AND {raw} NOT LIKE '%SHORT%'"
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
    """One-shot: divide option unit_price by 100 when cash ≈ price × qty.

    Wealthsimple option `amount` is contract cash. Correct per-share price is
    amount / (qty × 100). The old `unit_price > 20` heuristic left cheap
    contracts 100× high. Sync never replaces existing rows. Already-correct
    rows (cash ≈ price × qty × 100) are left alone. Caller runs this only
    when meta option_unit_price_scale_v1 is missing, then stamps the key.
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
            _ready(conn)
            _relabel_when_rows_changed(conn)
            stamped = conn.execute(
                "SELECT value FROM meta WHERE key = ?",
                (OPTION_UNIT_PRICE_SCALE_META,),
            ).fetchone()
            if not stamped:
                _scale_option_unit_prices(conn)
                conn.execute(
                    "INSERT INTO meta(key, value) VALUES (?, ?) "
                    "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    (OPTION_UNIT_PRICE_SCALE_META, "1"),
                )
            conn.commit()
        finally:
            conn.close()


def get_meta(key, default=""):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
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
            _ready(conn)
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
            _ready(conn)
            row = conn.execute("SELECT COUNT(*) AS n FROM activities").fetchone()
            return int(row["n"] if row else 0)
        finally:
            conn.close()


def canonical_ids():
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
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
            _ready(conn)
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


PULL_OVERLAP_DAYS = 14


def incremental_start_date():
    """Date bound for a daily pull: PULL_OVERLAP_DAYS before the newest stored
    Wealthsimple row. Wealthsimple files a row under the day it belongs to, not
    the day it appears: a dividend paid on the 8th can show up on the 9th, after
    a card purchase from the evening of the 8th has already been stored under
    the 9th. A window starting at the newest stored day would never see it.
    Rows already stored are dropped by canonical id, so the overlap costs a few
    pages and nothing else. Empty table must not call this for a full walk."""
    newest = newest_ws_occurred_at()
    if not newest:
        return ""
    day = newest.split("T", 1)[0][:10]
    try:
        start = datetime.strptime(day, "%Y-%m-%d") - timedelta(days=PULL_OVERLAP_DAYS)
    except ValueError:
        return day
    return start.strftime("%Y-%m-%d")


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
            _ready(conn)
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
            _ready(conn)
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
            _ready(conn)
            conn.execute(
                "UPDATE activities SET canonical_id = ? WHERE id = ? "
                "AND (canonical_id IS NULL OR canonical_id = '')",
                (cid, activity_id),
            )
            conn.commit()
            return conn.total_changes > 0
        finally:
            conn.close()


_INSERT_COLUMNS = (
    "id", "canonical_id", "occurred_at", "transaction_date", "settlement_date",
    "account_id", "book_id", "fifo_id", "account_type", "activity_type",
    "activity_sub_type", "description", "direction", "symbol", "name", "currency",
    "quantity", "unit_price", "commission", "net_cash_amount", "category", "balance",
    "source", "raw_type", "aft_type", "counter_symbol", "security_id",
)
# What Wealthsimple revises on a row of its own: a dividend announced as a
# placeholder on the record date (no cash, dated that day) becomes the paid
# dividend on pay day, under the same canonical id. Identity, account and the
# stored id stay.
_REVISABLE_COLUMNS = (
    "occurred_at", "transaction_date", "settlement_date", "activity_type", "activity_sub_type",
    "description", "direction", "symbol", "name", "currency", "quantity", "unit_price",
    "commission", "net_cash_amount", "category", "raw_type", "aft_type", "counter_symbol",
)


def _differs(a, b):
    if isinstance(a, float) or isinstance(b, float):
        try:
            return abs(float(a or 0) - float(b or 0)) > 1e-9
        except (TypeError, ValueError):
            return True
    return (a or "") != (b or "")


def _revise_wealthsimple_row(cid, row):
    """Replace the stored copy of a Wealthsimple row with Wealthsimple's current
    version when a revisable field changed. True when something was written."""
    incoming = dict(zip(_INSERT_COLUMNS, _insert_params(row, "", cid)))
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            stored = conn.execute("SELECT * FROM activities WHERE canonical_id = ?", (cid,)).fetchone()
            if not stored:
                return False
            changed = [c for c in _REVISABLE_COLUMNS if _differs(incoming[c], stored[c])]
            if not changed:
                return False
            if incoming["security_id"] and not stored["security_id"]:
                changed.append("security_id")
            sets = ", ".join("%s = ?" % c for c in changed)
            conn.execute("UPDATE activities SET %s WHERE canonical_id = ?" % sets, [incoming[c] for c in changed] + [cid])
            conn.commit()
            return True
        finally:
            conn.close()


def apply_wealthsimple_mapped(rows):
    """Insert if canonicalId is new. A known row is replaced only when
    Wealthsimple itself has revised it (see _REVISABLE_COLUMNS); Bagholder
    never edits a row on its own.

    Linking a typed/CSV row: exactly one field match stamps canonicalId.
    None → insert. Several → do not guess; insert the Wealthsimple row.
    """
    inserted = 0
    linked = 0
    skipped = 0
    revised = 0
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
            if _revise_wealthsimple_row(cid, row):
                revised += 1
                continue
            sid = _s(row.get("securityId") or row.get("security_id")).strip()
            if sid:
                with _lock:
                    conn = _connect()
                    try:
                        _ready(conn)
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
    return {"inserted": inserted, "linked": linked, "skipped": skipped, "revised": revised}


def merge_local_rows(rows):
    """CSV / typed merge on date, account, symbol, quantity, price, cash — not id."""
    stored = []
    added = 0
    duplicates = 0
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
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
            _ready(conn)
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
                    "net_liquidation_value, margin_account_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    (
                        aid,
                        _s(acc.get("nickname")),
                        _s(acc.get("unifiedAccountType") or acc.get("unified_account_type")),
                        _s(acc.get("currency")),
                        _s(acc.get("status")),
                        _s(acc.get("type")),
                        nlv,
                        _s(acc.get("marginAccountId")),
                    ),
                )
            conn.commit()
        finally:
            conn.close()


def replace_balances(balances):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
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
            _ready(conn)
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


def replace_margin(rows):
    """Wealthsimple's margin figures per account, replaced whole on every read:
    buying power (Margin available) with its currency, or the reason it was
    unavailable. Accounts that answer nothing are not rows."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute("DELETE FROM margin")
            now = _now_iso()
            for m in rows or []:
                if not isinstance(m, dict) or not _s(m.get("accountId")):
                    continue
                conn.execute(
                    "INSERT INTO margin (account_id, buying_power, currency, unavailable, fetched_at) VALUES (?, ?, ?, ?, ?)",
                    (_s(m.get("accountId")), _num(m.get("buyingPower"), None), _s(m.get("currency")) or "CAD", _s(m.get("unavailable")), _s(m.get("fetchedAt")) or now),
                )
            conn.commit()
        finally:
            conn.close()


def upsert_nav(points):
    """Insert or update daily-value rows. Does not delete existing days."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            _write_nav_points(conn, points)
            conn.commit()
        finally:
            conn.close()


def replace_nav(points):
    """Replace the whole nav_history table from identity-wide + per-nickname series."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
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



def _ensure_quote_columns(conn):
    cols = {r["name"] for r in conn.execute("PRAGMA table_info(quotes)").fetchall()}
    for col in ("price_change", "percent_change", "prev_close"):
        if col not in cols:
            conn.execute("ALTER TABLE quotes ADD COLUMN %s REAL" % col)


def _ensure_order_columns(conn):
    cols = {r["name"] for r in conn.execute("PRAGMA table_info(orders)").fetchall()}
    for col, typ in (("source", "TEXT"), ("ws_status", "TEXT"), ("filled_qty", "REAL"), ("avg_fill", "REAL"), ("submitted_at", "TEXT"), ("expires_at", "TEXT"), ("parent_id", "TEXT"), ("role", "TEXT")):
        if col not in cols:
            conn.execute("ALTER TABLE orders ADD COLUMN %s %s" % (col, typ))
    bcols = {r["name"] for r in conn.execute("PRAGMA table_info(brackets)").fetchall()}
    for col, typ in (("seen_held", "INTEGER"), ("missed_at", "TEXT")):
        if col not in bcols:
            conn.execute("ALTER TABLE brackets ADD COLUMN %s %s" % (col, typ))


def _ensure_account_columns(conn):
    cols = {r["name"] for r in conn.execute("PRAGMA table_info(accounts)").fetchall()}
    if "margin_account_id" not in cols:
        conn.execute("ALTER TABLE accounts ADD COLUMN margin_account_id TEXT")


def _migrate_spy_meta(conn):
    """One-shot: copy the legacy meta.spy_by_date map into benchmark_prices."""
    row = conn.execute(
        "SELECT 1 FROM benchmark_prices WHERE symbol = ? LIMIT 1", (BENCHMARK_SYMBOL,)
    ).fetchone()
    if row:
        return
    raw = conn.execute("SELECT value FROM meta WHERE key = 'spy_by_date'").fetchone()
    if not raw or not raw["value"]:
        return
    try:
        data = json.loads(raw["value"])
    except ValueError:
        return
    if not isinstance(data, dict):
        return
    for day, px in data.items():
        d = _s(day).strip()[:10]
        v = _num(px, None)
        if len(d) != 10 or v is None or v <= 0:
            continue
        conn.execute(
            "INSERT OR IGNORE INTO benchmark_prices(symbol, date, close) VALUES (?, ?, ?)",
            (BENCHMARK_SYMBOL, d, v),
        )


def _clean_date_map(raw):
    out = {}
    if not isinstance(raw, dict):
        return out
    for key, val in raw.items():
        d = _s(key).strip()[:10]
        if len(d) != 10 or d[4] != "-" or d[7] != "-":
            continue
        v = _num(val, None)
        if v is None or v <= 0:
            continue
        out[d] = v
    return out


def fx_rates(pair=FX_PAIR):
    """date -> units of CAD per 1 unit of the foreign currency."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            rows = conn.execute(
                "SELECT date, rate FROM fx_rates WHERE pair = ? ORDER BY date", (pair,)
            ).fetchall()
            return {r["date"]: r["rate"] for r in rows}
        finally:
            conn.close()


def fx_last_date(pair=FX_PAIR):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            row = conn.execute(
                "SELECT MAX(date) AS d FROM fx_rates WHERE pair = ?", (pair,)
            ).fetchone()
            return (row["d"] if row else "") or ""
        finally:
            conn.close()


def upsert_fx_rates(mapping, pair=FX_PAIR):
    clean = _clean_date_map(mapping)
    if not clean:
        return 0
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.executemany(
                "INSERT OR IGNORE INTO fx_rates(pair, date, rate) VALUES (?, ?, ?)",
                [(pair, d, v) for d, v in sorted(clean.items())],
            )
            conn.commit()
            return len(clean)
        finally:
            conn.close()


def benchmark_prices(symbol=BENCHMARK_SYMBOL):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            rows = conn.execute(
                "SELECT date, close FROM benchmark_prices WHERE symbol = ? ORDER BY date",
                (symbol,),
            ).fetchall()
            return {r["date"]: r["close"] for r in rows}
        finally:
            conn.close()


def benchmark_last_date(symbol=BENCHMARK_SYMBOL):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            row = conn.execute(
                "SELECT MAX(date) AS d FROM benchmark_prices WHERE symbol = ?", (symbol,)
            ).fetchone()
            return (row["d"] if row else "") or ""
        finally:
            conn.close()


def upsert_benchmark_prices(mapping, symbol=BENCHMARK_SYMBOL):
    clean = _clean_date_map(mapping)
    if not clean:
        return 0
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.executemany(
                "INSERT OR IGNORE INTO benchmark_prices(symbol, date, close) VALUES (?, ?, ?)",
                [(symbol, d, v) for d, v in sorted(clean.items())],
            )
            conn.commit()
            return len(clean)
        finally:
            conn.close()


def distributions():
    """symbol -> [{exDate, payDate, amount, currency}] newest first (public record)."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            out = {}
            for r in conn.execute("SELECT * FROM distributions ORDER BY symbol, ex_date DESC").fetchall():
                out.setdefault(r["symbol"], []).append(
                    {"exDate": r["ex_date"], "payDate": r["pay_date"] or "", "amount": r["amount"], "currency": r["currency"] or ""}
                )
            return out
        finally:
            conn.close()


def upsert_distributions(symbol, rows, source="tmx"):
    sym = _s(symbol).strip().upper()
    if not sym:
        return 0
    clean = []
    for r in rows or []:
        ex = _s(r.get("exDate"))[:10]
        amt = _num(r.get("amount"), None)
        if len(ex) != 10 or amt is None or amt <= 0:
            continue
        clean.append((sym, ex, _s(r.get("payDate"))[:10] or None, amt, _s(r.get("currency")) or None, source))
    if not clean:
        return 0
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.executemany(
                "INSERT INTO distributions(symbol, ex_date, pay_date, amount, currency, source) VALUES (?, ?, ?, ?, ?, ?) "
                "ON CONFLICT(symbol, ex_date, source) DO UPDATE SET pay_date = excluded.pay_date, amount = excluded.amount, currency = excluded.currency",
                clean,
            )
            conn.commit()
            return len(clean)
        finally:
            conn.close()


def quotes():
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            out = {}
            for r in conn.execute("SELECT * FROM quotes").fetchall():
                out[r["symbol"]] = {
                    "price": r["price"],
                    "priceChange": r["price_change"],
                    "percentChange": r["percent_change"],
                    "prevClose": r["prev_close"],
                    "dividendAmount": r["dividend_amount"],
                    "dividendFrequency": r["dividend_frequency"] or "",
                    "exDividendDate": r["ex_dividend_date"] or "",
                    "source": r["source"] or "",
                    "fetchedAt": r["fetched_at"] or "",
                }
            return out
        finally:
            conn.close()


def upsert_quote(symbol, rec, source="tmx"):
    sym = _s(symbol).strip().upper()
    if not sym or not isinstance(rec, dict):
        return
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute(
                "INSERT INTO quotes(symbol, price, price_change, percent_change, prev_close, dividend_amount, "
                "dividend_frequency, ex_dividend_date, source, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) "
                "ON CONFLICT(symbol) DO UPDATE SET price = excluded.price, price_change = excluded.price_change, "
                "percent_change = excluded.percent_change, prev_close = excluded.prev_close, "
                "dividend_amount = COALESCE(excluded.dividend_amount, quotes.dividend_amount), "
                "dividend_frequency = CASE WHEN excluded.dividend_frequency = '' THEN quotes.dividend_frequency ELSE excluded.dividend_frequency END, "
                "ex_dividend_date = CASE WHEN excluded.ex_dividend_date = '' THEN quotes.ex_dividend_date ELSE excluded.ex_dividend_date END, "
                "source = excluded.source, fetched_at = excluded.fetched_at",
                (
                    sym,
                    _num(rec.get("price"), None),
                    _num(rec.get("priceChange"), None),
                    _num(rec.get("percentChange"), None),
                    _num(rec.get("prevClose"), None),
                    _num(rec.get("dividendAmount"), None),
                    _s(rec.get("dividendFrequency")),
                    _s(rec.get("exDividendDate"))[:10],
                    source,
                    _s(rec.get("fetchedAt")) or _now_iso(),
                ),
            )
            conn.commit()
        finally:
            conn.close()


def quote_fetched_at():
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            return {r["symbol"]: r["fetched_at"] or "" for r in conn.execute("SELECT symbol, fetched_at FROM quotes").fetchall()}
        finally:
            conn.close()


def distributions_fetched_at():
    """symbol -> when its declared distribution record was last fetched."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            return {r["symbol"]: r["fetched_at"] or "" for r in conn.execute("SELECT symbol, fetched_at FROM distribution_fetches").fetchall()}
        finally:
            conn.close()


def price_history(symbol, start="", end=""):
    """Daily bars for one symbol, oldest first: [{date, open, high, low, close, volume}]."""
    sym = _s(symbol).strip().upper()
    if not sym:
        return []
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            rows = conn.execute(
                "SELECT date, open, high, low, close, volume FROM price_history WHERE symbol = ? AND date >= ? AND date <= ? ORDER BY date",
                (sym, _s(start)[:10] or "0000-01-01", _s(end)[:10] or "9999-12-31"),
            ).fetchall()
            return [{"date": r["date"], "open": r["open"], "high": r["high"], "low": r["low"], "close": r["close"], "volume": r["volume"]} for r in rows]
        finally:
            conn.close()


def upsert_price_history(symbol, bars, source=""):
    """Closed days are written once and never rewritten; the newest stored day may
    be replaced, since a source can hand back a bar for a session still in progress."""
    sym = _s(symbol).strip().upper()
    clean = []
    for b in bars or []:
        d = _s(b.get("date"))[:10]
        close = _num(b.get("close"), None)
        if len(d) != 10 or d[4] != "-" or close is None or close <= 0:
            continue
        clean.append((sym, d, _num(b.get("open"), None), _num(b.get("high"), None), _num(b.get("low"), None), close, _num(b.get("volume"), None), _s(source)))
    if not sym or not clean:
        return 0
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            newest = conn.execute("SELECT MAX(date) FROM price_history WHERE symbol = ?", (sym,)).fetchone()[0] or ""
            conn.executemany("INSERT OR IGNORE INTO price_history(symbol, date, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?, ?)", clean)
            if newest:
                conn.executemany(
                    "UPDATE price_history SET open = ?, high = ?, low = ?, close = ?, volume = ?, source = ? WHERE symbol = ? AND date = ?",
                    [(c[2], c[3], c[4], c[5], c[6], c[7], c[0], c[1]) for c in clean if c[1] == newest],
                )
            conn.commit()
            return len(clean)
        finally:
            conn.close()


def history_fetch(symbol):
    """{start, fetchedAt} of the last history fetch for a symbol, or None."""
    sym = _s(symbol).strip().upper()
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            r = conn.execute("SELECT start, fetched_at FROM history_fetches WHERE symbol = ?", (sym,)).fetchone()
            return {"start": r["start"], "fetchedAt": r["fetched_at"]} if r else None
        finally:
            conn.close()


def mark_history_fetched(symbol, start, when):
    sym = _s(symbol).strip().upper()
    if not sym or not when:
        return
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute(
                "INSERT INTO history_fetches(symbol, start, fetched_at) VALUES (?, ?, ?) ON CONFLICT(symbol) DO UPDATE SET start = MIN(history_fetches.start, excluded.start), fetched_at = excluded.fetched_at",
                (sym, _s(start)[:10], _s(when)),
            )
            conn.commit()
        finally:
            conn.close()


def _migrate_history_sources(conn):
    """Runs once when the chart's history sources change. Bars from a source that
    gave closes only (CoinGecko) are dropped; bars from a source no longer used
    for history (Cboe Canada's feed, real but three months deep) are kept; the
    fetch stamps of every symbol either source served are dropped, so the chart
    refetches the whole span from the source that replaced it."""
    done = conn.execute("SELECT value FROM meta WHERE key = 'history_sources_migrated'").fetchone()
    if done:
        return
    conn.execute("DELETE FROM price_history WHERE source = 'coingecko'")
    conn.execute("DELETE FROM history_fetches WHERE symbol NOT IN (SELECT DISTINCT symbol FROM price_history WHERE source NOT IN ('coingecko', 'cboe_ca'))")
    conn.execute("DELETE FROM price_bars WHERE source = 'coingecko'")
    conn.execute("DELETE FROM bar_fetches WHERE symbol NOT IN (SELECT DISTINCT symbol FROM price_bars)")
    # a stamp claiming a span its bars begin well after is dropped, so the chain
    # is asked again for the earlier days
    conn.execute(
        "DELETE FROM history_fetches WHERE symbol IN (SELECT h.symbol FROM history_fetches h JOIN "
        "(SELECT symbol, MIN(date) AS first FROM price_history GROUP BY symbol) p ON p.symbol = h.symbol "
        "WHERE julianday(p.first) - julianday(h.start) > 7)"
    )
    conn.execute(
        "DELETE FROM bar_fetches WHERE (symbol, tf) IN (SELECT b.symbol, b.tf FROM bar_fetches b JOIN "
        "(SELECT symbol, tf, MIN(ts) AS first FROM price_bars GROUP BY symbol, tf) p ON p.symbol = b.symbol AND p.tf = b.tf "
        "WHERE p.first - b.start_ts > 7 * 86400)"
    )
    conn.execute("INSERT INTO meta(key, value) VALUES ('history_sources_migrated', '1')")
    conn.commit()


def _ensure_bar_columns(conn):
    cols = {r[1] for r in conn.execute("PRAGMA table_info(price_bars)").fetchall()}
    if cols and "open" not in cols:
        conn.execute("DROP TABLE price_bars")
        conn.execute("DELETE FROM bar_fetches")
        conn.execute(
            "CREATE TABLE price_bars (symbol TEXT NOT NULL, tf TEXT NOT NULL, ts INTEGER NOT NULL, open REAL, high REAL, low REAL, "
            "close REAL NOT NULL, volume REAL, source TEXT NOT NULL DEFAULT '', PRIMARY KEY (symbol, tf, ts))"
        )
        conn.commit()


def price_bars(symbol, tf, start_ts=0, end_ts=2 ** 40):
    """Intraday bars for one symbol and timeframe, oldest first: [{time, open, high, low, close, volume}]."""
    sym = _s(symbol).strip().upper()
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            rows = conn.execute("SELECT ts, open, high, low, close, volume FROM price_bars WHERE symbol = ? AND tf = ? AND ts >= ? AND ts <= ? ORDER BY ts", (sym, _s(tf), int(start_ts), int(end_ts))).fetchall()
            return [{"time": r["ts"], "open": r["open"], "high": r["high"], "low": r["low"], "close": r["close"], "volume": r["volume"]} for r in rows]
        finally:
            conn.close()


def upsert_price_bars(symbol, tf, bars, source=""):
    """Closed bars are written once; the newest stored bar may be replaced."""
    sym = _s(symbol).strip().upper()
    clean = []
    for b in bars or []:
        close = _num(b.get("close"), None)
        if b.get("time") is None or not close or close <= 0:
            continue
        clean.append((sym, _s(tf), int(b["time"]), _num(b.get("open"), None), _num(b.get("high"), None), _num(b.get("low"), None), close, _num(b.get("volume"), None), _s(source)))
    if not sym or not clean:
        return 0
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            newest = conn.execute("SELECT MAX(ts) FROM price_bars WHERE symbol = ? AND tf = ?", (sym, _s(tf))).fetchone()[0]
            conn.executemany("INSERT OR IGNORE INTO price_bars(symbol, tf, ts, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)", clean)
            if newest is not None:
                conn.executemany(
                    "UPDATE price_bars SET open = ?, high = ?, low = ?, close = ?, volume = ?, source = ? WHERE symbol = ? AND tf = ? AND ts = ?",
                    [(c[3], c[4], c[5], c[6], c[7], c[8], c[0], c[1], c[2]) for c in clean if c[2] == newest],
                )
            conn.commit()
            return len(clean)
        finally:
            conn.close()


def bar_fetch(symbol, tf):
    sym = _s(symbol).strip().upper()
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            r = conn.execute("SELECT start_ts, fetched_at FROM bar_fetches WHERE symbol = ? AND tf = ?", (sym, _s(tf))).fetchone()
            return {"startTs": r["start_ts"], "fetchedAt": r["fetched_at"]} if r else None
        finally:
            conn.close()


def mark_bars_fetched(symbol, tf, start_ts, when):
    sym = _s(symbol).strip().upper()
    if not sym or not when:
        return
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute(
                "INSERT INTO bar_fetches(symbol, tf, start_ts, fetched_at) VALUES (?, ?, ?, ?) ON CONFLICT(symbol, tf) DO UPDATE SET start_ts = MIN(bar_fetches.start_ts, excluded.start_ts), fetched_at = excluded.fetched_at",
                (sym, _s(tf), int(start_ts), _s(when)),
            )
            conn.commit()
        finally:
            conn.close()


def mark_distributions_fetched(symbol, when):
    sym = _s(symbol).strip().upper()
    if not sym or not when:
        return
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute(
                "INSERT INTO distribution_fetches(symbol, fetched_at) VALUES (?, ?) ON CONFLICT(symbol) DO UPDATE SET fetched_at = excluded.fetched_at",
                (sym, _s(when)),
            )
            conn.commit()
        finally:
            conn.close()


_CANADIAN_EXCHANGES = ("TSX", "TSX-V", "TSXV", "CSE", "CBOE CANADA", "NEO", "ALPHA EXCHANGE", "")


def dividend_symbols():
    """Symbols that have paid a dividend, with the listing exchange when known."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            rows = conn.execute(
                "SELECT DISTINCT a.symbol AS symbol, a.currency AS currency, s.primary_exchange AS exchange "
                "FROM activities a LEFT JOIN securities s ON s.id = a.security_id "
                "WHERE a.category = 'dividend' AND IFNULL(a.symbol, '') != ''"
            ).fetchall()
            out = []
            seen = set()
            for r in rows:
                sym = _s(r["symbol"]).strip().upper()
                if not sym or sym in seen:
                    continue
                seen.add(sym)
                out.append({"symbol": sym, "currency": _s(r["currency"]), "exchange": _s(r["exchange"]).strip()})
            return out
        finally:
            conn.close()


BENCHMARK_SYMBOLS = ("SP500", "TSX", "TSX60")


def market_data():
    return {
        "fx": fx_rates(),
        "benchmark": benchmark_prices(),
        "benchmarks": {sym: benchmark_prices(sym) for sym in BENCHMARK_SYMBOLS},
        "distributions": distributions(),
        "quotes": quotes(),
    }


_GRADES = ("A", "B", "C", "F")


def _clean_journal_entry(val):
    if not isinstance(val, dict):
        return None
    thesis = _s(val.get("thesis"))
    grade = _s(val.get("grade")).strip().upper()
    if grade not in _GRADES:
        grade = ""
    tags = []
    raw_tags = val.get("tags")
    if isinstance(raw_tags, str):
        raw_tags = raw_tags.split(",")
    if isinstance(raw_tags, list):
        for t in raw_tags:
            s = _s(t).strip()
            if s and s not in tags:
                tags.append(s)
    if not thesis and not grade and not tags:
        return None
    return {"thesis": thesis, "tags": tags, "grade": grade}


def _clean_journal(raw):
    out = {}
    if not isinstance(raw, dict):
        return out
    for key, val in raw.items():
        kid = _s(key).strip()
        entry = _clean_journal_entry(val)
        if kid and entry:
            out[kid] = entry
    return out


def journal():
    """v2 journal: {tradeId or positionId: {thesis, tags, grade}}."""
    raw = get_meta(JOURNAL_META)
    if not raw:
        return {}
    try:
        return _clean_journal(json.loads(raw))
    except ValueError:
        return {}


def save_journal(entries):
    clean = _clean_journal(entries if isinstance(entries, dict) else {})
    set_meta(JOURNAL_META, json.dumps(clean))
    return clean


def save_journal_entry(key, entry):
    """Merge one entry. An entry with no thesis, grade, or tags deletes the key."""
    kid = _s(key).strip()
    if not kid:
        return journal()
    current = journal()
    clean = _clean_journal_entry(entry)
    if clean:
        current[kid] = clean
    else:
        current.pop(kid, None)
    set_meta(JOURNAL_META, json.dumps(current))
    return current


SYNC_META_KEYS = ("synced_at", "last_activity_pull", "security_id_backfill_done")


def data_summary():
    """Row counts the Data & storage dialog shows before a wipe."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            count = lambda sql: int(conn.execute(sql).fetchone()[0] or 0)
            journal_raw = conn.execute("SELECT value FROM meta WHERE key = ?", (JOURNAL_META,)).fetchone()
            try:
                journal_n = len(json.loads(journal_raw["value"])) if journal_raw and journal_raw["value"] else 0
            except ValueError:
                journal_n = 0
            first = conn.execute("SELECT MIN(transaction_date), MAX(transaction_date) FROM activities").fetchone()
            return {
                "path": str(db_path()),
                "activities": count("SELECT COUNT(*) FROM activities"),
                "firstActivity": first[0] or "",
                "lastActivity": first[1] or "",
                "accounts": count("SELECT COUNT(*) FROM accounts"),
                "balances": count("SELECT COUNT(*) FROM balances"),
                "navDays": count("SELECT COUNT(*) FROM nav_history"),
                "securities": count("SELECT COUNT(*) FROM securities"),
                "journal": journal_n,
                "fxDays": count("SELECT COUNT(*) FROM fx_rates"),
                "benchmarkDays": count("SELECT COUNT(*) FROM benchmark_prices"),
                "syncedAt": get_meta("synced_at"),
            }
        finally:
            conn.close()


def clear_synced_data(keep_journal=True, keep_market=True):
    """Wipe everything Wealthsimple sync wrote so the next sync starts from zero.

    Activities, accounts, balances, NAV history, securities, manual trade
    groups and the sync bookmarks go. The journal (grades, tags, theses) and
    the downloaded market data (FX, benchmark, distributions, quotes, price
    history and bars, with their fetch records) are kept unless told otherwise.
    The Wealthsimple login is not this function's business."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            for table in ("activities", "accounts", "balances", "margin", "nav_history", "securities", "grouped_trades"):
                conn.execute("DELETE FROM %s" % table)
            keys = list(SYNC_META_KEYS) + ["trade_groups", "trade_notes"]
            if not keep_journal:
                keys.append(JOURNAL_META)
            conn.executemany("DELETE FROM meta WHERE key = ?", [(k,) for k in keys])
            if not keep_market:
                for table in ("fx_rates", "benchmark_prices", "distributions", "distribution_fetches", "quotes",
                              "price_history", "history_fetches", "price_bars", "bar_fetches"):
                    conn.execute("DELETE FROM %s" % table)
                conn.execute("DELETE FROM meta WHERE key IN ('spy_by_date', 'market_attempt_at')")
                for prefix in ("bars_miss:", "bars_source:", "coinbase_product:", "coingecko_id:", "tmx_form:", "yahoo_miss:"):
                    conn.execute("DELETE FROM meta WHERE key LIKE ?", (prefix + "%",))
            conn.commit()
        finally:
            conn.close()
    return data_summary()


def book_version():
    """Fingerprint of what the FIFO match reads: the activity rows and the securities."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            parts = []
            for sql in (
                "SELECT COUNT(*), MAX(COALESCE(occurred_at, transaction_date)) FROM activities",
                "SELECT COUNT(*), MAX(fetched_at) FROM securities",
            ):
                row = conn.execute(sql).fetchone()
                parts.append("%s:%s" % (row[0], row[1]))
            return "|".join(parts)
        finally:
            conn.close()


TILES_META = "market_tiles"


def _tiles_from(raw):
    """The market tiles row as saved: [{symbol, exchange}] in order, or None when never saved."""
    if not raw:
        return None
    try:
        rows = json.loads(raw)
    except ValueError:
        return None
    out = []
    for r in rows if isinstance(rows, list) else []:
        if isinstance(r, dict) and _s(r.get("symbol")).strip():
            out.append({"symbol": _s(r.get("symbol")).strip().upper(), "exchange": _s(r.get("exchange")).strip().upper()})
    return out


def tiles():
    return _tiles_from(get_meta(TILES_META))


def save_tiles(rows):
    """The market tiles row, in order; saving it is what the Markets tab's plus, cross and drag do."""
    clean = _tiles_from(json.dumps(list(rows or []))) or []
    set_meta(TILES_META, json.dumps(clean))
    return clean


def status_counts():
    """What the header needs: how many activities and accounts are stored, and
    when the last sync finished. The page asks for this every thirty seconds;
    reading the tables themselves to count their rows cost a tenth of a second
    and thirty megabytes for two numbers and a date."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            acts = conn.execute("SELECT COUNT(*) AS n FROM activities").fetchone()["n"]
            accounts = conn.execute("SELECT COUNT(*) AS n FROM accounts").fetchone()["n"]
            row = conn.execute("SELECT value FROM meta WHERE key = 'synced_at'").fetchone()
            return {
                "activityCount": int(acts or 0),
                "accountCount": int(accounts or 0),
                "syncedAt": (row["value"] if row else "") or "",
            }
        finally:
            conn.close()


# The tables and meta keys the derived model reads. `quotes` is kept apart:
# prices move every minute, and everything else in the model stands still while
# they do, so a build can reuse what it already has.
_VERSION_SQL = (
    "SELECT COUNT(*), MAX(COALESCE(occurred_at, transaction_date)) FROM activities",
    "SELECT COUNT(*), MAX(date) FROM nav_history",
    "SELECT COUNT(*), MAX(date) FROM fx_rates",
    "SELECT COUNT(*), MAX(date) FROM benchmark_prices",
    "SELECT COUNT(*), MAX(ex_date) FROM distributions",
    "SELECT COUNT(*), MAX(fetched_at) FROM securities",
    "SELECT COUNT(*), SUM(quantity) FROM balances",
    "SELECT COUNT(*), MAX(id) FROM accounts",
    "SELECT COUNT(*), MAX(fetched_at) FROM margin",
    "SELECT COUNT(*), MAX(fetched_at) FROM exposures",
    "SELECT COUNT(*), MAX(added_at) FROM watchlist",
    "SELECT COUNT(*), MAX(fetched_at) FROM news",
    "SELECT COUNT(*), MAX(fetched_at) FROM universes",
    "SELECT COUNT(*), SUM(COALESCE(net_liquidation_value, 0)) FROM accounts",
)
# the prices themselves, not only the stamp: two quotes written in the same
# second used to leave the fingerprint unchanged, and the page kept the old price
_QUOTES_SQL = "SELECT COUNT(*), MAX(fetched_at), TOTAL(price) FROM quotes"
_VERSION_META = ("synced_at", "trade_groups", "trade_notes", JOURNAL_META, TILES_META)


def versions():
    """(everything, everything but the quotes) in one pass.

    The first says whether the derived model is current at all. The second says
    whether anything but a price has moved: when it has not, the match, the
    closed trades, the cashflow and the equity curve are still good and only the
    open positions have to be marked again."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            parts = []
            for sql in _VERSION_SQL:
                row = conn.execute(sql).fetchone()
                parts.append("%s:%s" % (row[0], row[1]))
            for key in _VERSION_META:
                row = conn.execute("SELECT value FROM meta WHERE key = ?", (key,)).fetchone()
                val = (row["value"] if row else "") or ""
                parts.append("%s:%s:%s" % (key, len(val), hash(val)))
            core = "|".join(parts)
            row = conn.execute(_QUOTES_SQL).fetchone()
            return core + "|q:%s:%s:%s" % (row[0], row[1], row[2]), core
        finally:
            conn.close()


def data_version():
    """Cheap fingerprint of everything the derived model depends on."""
    return versions()[0]


def core_version():
    """The same, without the quotes."""
    return versions()[1]


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
            _ready(conn)
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
            _ready(conn)
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
            _ready(conn)
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
            _ready(conn)
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


def snapshot(activities=True):
    ensure()
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            activities = _all_activities(conn) if activities else []
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
                        "marginAccountId": r["margin_account_id"] or "",
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
            margin = []
            for r in conn.execute("SELECT * FROM margin ORDER BY account_id").fetchall():
                margin.append(
                    {
                        "accountId": r["account_id"],
                        "buyingPower": r["buying_power"],
                        "currency": r["currency"] or "CAD",
                        "unavailable": r["unavailable"] or "",
                        "fetchedAt": r["fetched_at"] or "",
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
            "margin": margin,
            "exposures": {r["key"]: _exposure_from_row(r) for r in conn.execute("SELECT * FROM exposures").fetchall()},
                "watchlist": [_watch_from_row(r) for r in conn.execute("SELECT * FROM watchlist ORDER BY added_at, symbol").fetchall()],
                "news": [_news_from_row(r) for r in conn.execute("SELECT * FROM news ORDER BY published_at DESC, id").fetchall()],
                "universes": _universes(conn),
                "navHistory": nav,
                "navByAccount": nav_by_account,
                "syncedAt": synced,
                "tradeGroups": groups,
                "notes": notes,
                "tiles": _tiles_from(get_meta(TILES_META)),
                "securities": securities,
            }
        finally:
            conn.close()


# ---------------------------------------------------------------------------
# orders: every ticket the app submitted, with what was sent and what came back
# ---------------------------------------------------------------------------
ORDER_FIELDS = ("id", "createdAt", "accountId", "account", "securityId", "symbol", "currency", "side", "type", "quantity",
                "limitPrice", "stopPrice", "tif", "stopLoss", "takeProfit", "status", "wsOrderId", "error", "request", "updatedAt")


def _order_from_row(r):
    def js(v):
        if not v:
            return None
        try:
            return json.loads(v)
        except (TypeError, ValueError):
            return None
    return {
        "id": r["id"],
        "createdAt": r["created_at"],
        "accountId": r["account_id"],
        "account": r["account"] or "",
        "securityId": r["security_id"],
        "symbol": r["symbol"] or "",
        "currency": r["currency"] or "",
        "side": r["side"],
        "type": r["type"],
        "quantity": r["quantity"],
        "limitPrice": r["limit_price"],
        "stopPrice": r["stop_price"],
        "tif": r["tif"],
        "stopLoss": js(r["stop_loss"]),
        "takeProfit": js(r["take_profit"]),
        "status": r["status"],
        "wsOrderId": r["ws_order_id"] or "",
        "error": r["error"] or "",
        "request": js(r["request"]),
        "updatedAt": r["updated_at"] or "",
        "source": r["source"] or "bagholder",
        "wsStatus": r["ws_status"] or "",
        "filledQty": r["filled_qty"],
        "avgFill": r["avg_fill"],
        "submittedAt": r["submitted_at"] or "",
        "expiresAt": r["expires_at"] or "",
        "parentId": r["parent_id"] or "",
        "role": r["role"] or "entry",
    }


def insert_order(row):
    """A new ticket, written before anything is sent."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            now = _now_iso()
            conn.execute(
                "INSERT INTO orders (id, created_at, account_id, account, security_id, symbol, currency, side, type, quantity, "
                "limit_price, stop_price, tif, stop_loss, take_profit, status, ws_order_id, error, request, updated_at, "
                "source, ws_status, filled_qty, avg_fill, submitted_at, expires_at, parent_id, role) "
                "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    _s(row.get("id")), _s(row.get("createdAt")) or now, _s(row.get("accountId")), _s(row.get("account")),
                    _s(row.get("securityId")), _s(row.get("symbol")), _s(row.get("currency")), _s(row.get("side")), _s(row.get("type")),
                    _num(row.get("quantity")), _num(row.get("limitPrice"), None), _num(row.get("stopPrice"), None), _s(row.get("tif")),
                    json.dumps(row["stopLoss"]) if row.get("stopLoss") else None, json.dumps(row["takeProfit"]) if row.get("takeProfit") else None,
                    _s(row.get("status")), _s(row.get("wsOrderId")), _s(row.get("error")),
                    json.dumps(row["request"], sort_keys=True) if row.get("request") else None, now,
                    _s(row.get("source")) or "bagholder", _s(row.get("wsStatus")), _num(row.get("filledQty"), None), _num(row.get("avgFill"), None),
                    _s(row.get("submittedAt")), _s(row.get("expiresAt")), _s(row.get("parentId")), _s(row.get("role")) or "entry",
                ),
            )
            conn.commit()
        finally:
            conn.close()


def update_order(order_id, patch):
    """Status, Wealthsimple's order id, or an error on an existing ticket."""
    text = {"status": "status", "wsOrderId": "ws_order_id", "error": "error", "wsStatus": "ws_status", "submittedAt": "submitted_at", "expiresAt": "expires_at", "tif": "tif", "currency": "currency", "symbol": "symbol"}
    nums = {"filledQty": "filled_qty", "avgFill": "avg_fill", "quantity": "quantity", "limitPrice": "limit_price", "stopPrice": "stop_price"}
    sets, vals = [], []
    for k, col in text.items():
        if k in (patch or {}):
            sets.append(col + " = ?")
            vals.append(_s(patch[k]))
    for k, col in nums.items():
        if k in (patch or {}):
            sets.append(col + " = ?")
            vals.append(_num(patch[k], None))
    if not sets:
        return
    sets.append("updated_at = ?")
    vals.append(_now_iso())
    vals.append(_s(order_id))
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute("UPDATE orders SET " + ", ".join(sets) + " WHERE id = ?", vals)
            conn.commit()
        finally:
            conn.close()


def list_orders(limit=200):
    """Newest first."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            rows = conn.execute("SELECT * FROM orders ORDER BY created_at DESC, rowid DESC LIMIT ?", (int(limit),)).fetchall()
            return [_order_from_row(r) for r in rows]
        finally:
            conn.close()


def get_order(order_id):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            r = conn.execute("SELECT * FROM orders WHERE id = ?", (_s(order_id),)).fetchone()
            return _order_from_row(r) if r else None
        finally:
            conn.close()


# ---------------------------------------------------------------------------
# brackets: the stop loss and take profit Bagholder watches for an order
# ---------------------------------------------------------------------------
def _bracket_from_row(r):
    return {
        "id": r["id"], "orderId": r["order_id"], "createdAt": r["created_at"], "accountId": r["account_id"], "securityId": r["security_id"],
        "symbol": r["symbol"] or "", "currency": r["currency"] or "", "quantity": r["quantity"], "tif": r["tif"] or "DAY",
        "slKind": r["sl_kind"] or "", "slPrice": r["sl_price"], "slTrail": r["sl_trail"], "slTrailUnit": r["sl_trail_unit"] or "pct",
        "slOrderId": r["sl_order_id"] or "", "slNative": bool(r["sl_native"]), "slMode": r["sl_mode"] or "", "highWater": r["high_water"],
        "tpPrice": r["tp_price"], "tpOrderId": r["tp_order_id"] or "",
        "status": r["status"], "outcome": r["outcome"] or "", "error": r["error"] or "", "attempts": r["attempts"] or 0,
        "movedAt": r["moved_at"] or "", "armedAt": r["armed_at"] or "", "seenHeld": bool(r["seen_held"]), "missedAt": r["missed_at"] or "", "updatedAt": r["updated_at"] or "",
    }


BRACKET_TEXT = {"symbol": "symbol", "currency": "currency", "tif": "tif", "slKind": "sl_kind", "slTrailUnit": "sl_trail_unit", "slOrderId": "sl_order_id",
                "tpOrderId": "tp_order_id", "status": "status", "outcome": "outcome", "error": "error", "movedAt": "moved_at", "armedAt": "armed_at", "slMode": "sl_mode", "missedAt": "missed_at"}
BRACKET_NUM = {"quantity": "quantity", "slPrice": "sl_price", "slTrail": "sl_trail", "highWater": "high_water", "tpPrice": "tp_price", "attempts": "attempts", "slNative": "sl_native", "seenHeld": "seen_held"}


def insert_bracket(b):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            now = _now_iso()
            conn.execute(
                "INSERT INTO brackets (id, order_id, created_at, account_id, security_id, symbol, currency, quantity, tif, sl_kind, sl_price, sl_trail, "
                "sl_trail_unit, sl_order_id, sl_native, sl_mode, high_water, tp_price, tp_order_id, status, outcome, error, attempts, moved_at, armed_at, updated_at) "
                "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (_s(b.get("id")), _s(b.get("orderId")), _s(b.get("createdAt")) or now, _s(b.get("accountId")), _s(b.get("securityId")), _s(b.get("symbol")),
                 _s(b.get("currency")), _num(b.get("quantity"), None), _s(b.get("tif")) or "DAY", _s(b.get("slKind")), _num(b.get("slPrice"), None), _num(b.get("slTrail"), None),
                 _s(b.get("slTrailUnit")) or "pct", _s(b.get("slOrderId")), 1 if b.get("slNative") else 0, _s(b.get("slMode")), _num(b.get("highWater"), None), _num(b.get("tpPrice"), None),
                 _s(b.get("tpOrderId")), _s(b.get("status")) or "waiting", _s(b.get("outcome")), _s(b.get("error")), int(b.get("attempts") or 0), _s(b.get("movedAt")), _s(b.get("armedAt")), now),
            )
            conn.commit()
        finally:
            conn.close()


def update_bracket(bracket_id, patch):
    sets, vals = [], []
    for k, col in BRACKET_TEXT.items():
        if k in (patch or {}):
            sets.append(col + " = ?"); vals.append(_s(patch[k]))
    for k, col in BRACKET_NUM.items():
        if k in (patch or {}):
            v = patch[k]
            sets.append(col + " = ?"); vals.append(None if v is None else (int(bool(v)) if k in ("slNative", "seenHeld") else (int(v) if k == "attempts" else _num(v, None))))
    if not sets:
        return
    sets.append("updated_at = ?"); vals.append(_now_iso()); vals.append(_s(bracket_id))
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute("UPDATE brackets SET " + ", ".join(sets) + " WHERE id = ?", vals)
            conn.commit()
        finally:
            conn.close()


def list_brackets(statuses=None):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            if statuses:
                marks = ",".join("?" for _ in statuses)
                rows = conn.execute("SELECT * FROM brackets WHERE status IN (%s) ORDER BY created_at" % marks, list(statuses)).fetchall()
            else:
                rows = conn.execute("SELECT * FROM brackets ORDER BY created_at").fetchall()
            return [_bracket_from_row(r) for r in rows]
        finally:
            conn.close()


def get_bracket(bracket_id):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            r = conn.execute("SELECT * FROM brackets WHERE id = ?", (_s(bracket_id),)).fetchone()
            return _bracket_from_row(r) if r else None
        finally:
            conn.close()


def bracket_for_order(order_id):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            r = conn.execute("SELECT * FROM brackets WHERE order_id = ? ORDER BY created_at DESC LIMIT 1", (_s(order_id),)).fetchone()
            return _bracket_from_row(r) if r else None
        finally:
            conn.close()


def symbol_for_security(security_id):
    """The symbol the book uses for a security, from its activity rows: for an option
    contract that is the contract name (`QNC 20NOV26 3.00 CALL`), which the securities
    table does not carry. Empty when the book has no row for it."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            r = conn.execute("SELECT symbol FROM activities WHERE security_id = ? AND symbol IS NOT NULL AND symbol != '' ORDER BY occurred_at DESC LIMIT 1", (_s(security_id),)).fetchone()
            return _s(r["symbol"]) if r else ""
        finally:
            conn.close()


def replace_exposure(key, rec):
    """One exposure record: sectors and countries as {name: fraction}, the share of
    the holding they cover, the source and its as-of date."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute(
                "INSERT INTO exposures (key, sectors, countries, coverage, source, as_of, industry, error, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) "
                "ON CONFLICT(key) DO UPDATE SET sectors = excluded.sectors, countries = excluded.countries, coverage = excluded.coverage, source = excluded.source, "
                "as_of = excluded.as_of, industry = excluded.industry, error = excluded.error, fetched_at = excluded.fetched_at",
                (_s(key), json.dumps(rec.get("sectors") or {}), json.dumps(rec.get("countries") or {}), _num(rec.get("coverage"), 0.0), _s(rec.get("source")), _s(rec.get("asOf")),
                 _s(rec.get("industry")), _s(rec.get("error")), _now_iso()),
            )
            conn.commit()
        finally:
            conn.close()


def _exposure_from_row(r):
    def js(v):
        try:
            return json.loads(v) if v else {}
        except (TypeError, ValueError):
            return {}
    return {"sectors": js(r["sectors"]), "countries": js(r["countries"]), "coverage": r["coverage"] or 0.0, "source": r["source"] or "", "asOf": r["as_of"] or "",
            "industry": r["industry"] or "", "error": r["error"] or "", "fetchedAt": r["fetched_at"] or ""}


def exposure_record(key):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            r = conn.execute("SELECT * FROM exposures WHERE key = ?", (_s(key),)).fetchone()
            return _exposure_from_row(r) if r else None
        finally:
            conn.close()


# ---------------------------------------------------------------------------
# watchlist: listings the user follows without holding them
# ---------------------------------------------------------------------------
def _watch_from_row(r):
    return {"symbol": r["symbol"], "exchange": r["exchange"] or "", "name": r["name"] or "", "currency": r["currency"] or "",
            "securityId": r["security_id"] or "", "addedAt": r["added_at"] or ""}


def list_watchlist():
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            return [_watch_from_row(r) for r in conn.execute("SELECT * FROM watchlist ORDER BY added_at, symbol").fetchall()]
        finally:
            conn.close()


def add_watch(symbol, exchange="", name="", currency="", security_id="", now=None):
    """Follow a listing; adding one already followed keeps its place and fills in what was blank."""
    sym = _s(symbol).strip().upper()
    ex = _s(exchange).strip().upper()
    if not sym:
        return None
    when = _s(now) or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute("INSERT OR IGNORE INTO watchlist (symbol, exchange, name, currency, security_id, added_at) VALUES (?, ?, ?, ?, ?, ?)",
                         (sym, ex, _s(name), _s(currency).upper(), _s(security_id), when))
            conn.execute("UPDATE watchlist SET name = CASE WHEN COALESCE(name, '') = '' THEN ? ELSE name END, "
                         "currency = CASE WHEN COALESCE(currency, '') = '' THEN ? ELSE currency END, "
                         "security_id = CASE WHEN COALESCE(security_id, '') = '' THEN ? ELSE security_id END WHERE symbol = ? AND exchange = ?",
                         (_s(name), _s(currency).upper(), _s(security_id), sym, ex))
            conn.commit()
            return _watch_from_row(conn.execute("SELECT * FROM watchlist WHERE symbol = ? AND exchange = ?", (sym, ex)).fetchone())
        finally:
            conn.close()


def remove_watch(symbol, exchange=""):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            cur = conn.execute("DELETE FROM watchlist WHERE symbol = ? AND exchange = ?", (_s(symbol).strip().upper(), _s(exchange).strip().upper()))
            conn.commit()
            return cur.rowcount > 0
        finally:
            conn.close()


# ---------------------------------------------------------------------------
# news: the items read for each symbol, newest first
# ---------------------------------------------------------------------------
def news_key(symbol, exchange):
    return _s(symbol).strip().upper() + "@" + _s(exchange).strip().upper()


def _news_from_row(r):
    return {"id": r["id"], "symbol": r["symbol"], "exchange": r["exchange"] or "", "source": r["source"] or "", "headline": r["headline"] or "",
            "wire": r["wire"] or "", "url": r["url"] or "", "publishedAt": r["published_at"] or "", "fetchedAt": r["fetched_at"] or ""}


def replace_news(symbol, exchange, source, rows, now=None):
    """The wire's latest items for one listing, in place of what it had."""
    sym, ex = _s(symbol).strip().upper(), _s(exchange).strip().upper()
    when = _s(now.strftime("%Y-%m-%dT%H:%M:%SZ") if hasattr(now, "strftime") else now) or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute("DELETE FROM news WHERE symbol = ? AND exchange = ?", (sym, ex))
            conn.executemany("INSERT OR REPLACE INTO news (id, symbol, exchange, source, headline, wire, url, published_at, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                             [(_s(r.get("id")), sym, ex, _s(source), _s(r.get("headline")), _s(r.get("source")), _s(r.get("url")), _s(r.get("publishedAt")), when) for r in rows or [] if r.get("id")])
            conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)", ("news_fetched:" + news_key(sym, ex), when))
            conn.commit()
        finally:
            conn.close()


def news_fetched_at():
    """{symbol@venue: when its wire was last read}."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            return {r["key"][len("news_fetched:"):]: r["value"] for r in conn.execute("SELECT key, value FROM meta WHERE key LIKE 'news_fetched:%'").fetchall()}
        finally:
            conn.close()


def forget_news(symbol, exchange):
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute("DELETE FROM news WHERE symbol = ? AND exchange = ?", (_s(symbol).strip().upper(), _s(exchange).strip().upper()))
            conn.execute("DELETE FROM meta WHERE key = ?", ("news_fetched:" + news_key(symbol, exchange),))
            conn.commit()
        finally:
            conn.close()


def trim_news(keep):
    """Keep the newest `keep` items over every symbol."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute("DELETE FROM news WHERE rowid NOT IN (SELECT rowid FROM news ORDER BY published_at DESC, id LIMIT ?)", (int(keep),))
            conn.commit()
        finally:
            conn.close()


# ---------------------------------------------------------------------------
# universes: the market heatmaps' tiles, one set per key
# ---------------------------------------------------------------------------
def _universes(conn):
    out = {}
    for r in conn.execute("SELECT * FROM universes ORDER BY key, value DESC, symbol").fetchall():
        out.setdefault(r["key"], []).append({"symbol": r["symbol"], "name": r["name"] or "", "value": r["value"], "percentChange": r["percent_change"],
                                             "sector": r["sector"] or "", "country": r["country"] or "", "fetchedAt": r["fetched_at"] or ""})
    return out


def replace_universe(key, rows, now=None):
    when = _s(now) or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            conn.execute("DELETE FROM universes WHERE key = ?", (key,))
            conn.executemany("INSERT OR REPLACE INTO universes (key, symbol, name, value, percent_change, sector, country, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                             [(key, _s(r.get("symbol")), _s(r.get("name")), r.get("value"), r.get("percentChange"), _s(r.get("sector")), _s(r.get("country")), when) for r in rows or [] if r.get("symbol")])
            conn.commit()
        finally:
            conn.close()


def exposures_map():
    """Every record keyed by what it is for: a security id, or a share/fund key."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            return {r["key"]: _exposure_from_row(r) for r in conn.execute("SELECT * FROM exposures").fetchall()}
        finally:
            conn.close()


def sold_since(account_id, security_id, since_iso, symbol=""):
    """Shares sold in that account since a moment, from the activity feed: the sum of
    the Trade/SELL rows for the security (by id, else by symbol)."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            if _s(security_id):
                r = conn.execute("SELECT SUM(quantity) AS q FROM activities WHERE account_id = ? AND security_id = ? AND activity_type = 'Trade' AND activity_sub_type = 'SELL' AND occurred_at > ?",
                                 (_s(account_id), _s(security_id), _s(since_iso))).fetchone()
            else:
                r = conn.execute("SELECT SUM(quantity) AS q FROM activities WHERE account_id = ? AND symbol = ? AND activity_type = 'Trade' AND activity_sub_type = 'SELL' AND occurred_at > ?",
                                 (_s(account_id), _s(symbol), _s(since_iso))).fetchone()
            return float(r["q"]) if r is not None and r["q"] is not None else 0.0
        finally:
            conn.close()


def position_quantity(account_id, security_id):
    """Wealthsimple's balance for one security in one account, as last read; None when unknown."""
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            r = conn.execute("SELECT SUM(quantity) AS q FROM balances WHERE account_id = ? AND security_id = ?", (_s(account_id), _s(security_id))).fetchone()
            return None if r is None or r["q"] is None else float(r["q"])
        finally:
            conn.close()


def balances_count():
    with _lock:
        conn = _connect()
        try:
            _ready(conn)
            return int(conn.execute("SELECT COUNT(*) FROM balances").fetchone()[0])
        finally:
            conn.close()
