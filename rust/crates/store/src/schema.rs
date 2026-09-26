//! The SQLite schema and the migrations that bring an older database up to it.
//!
//! The DDL is kept in `sql/` and included here,
//! so the two cannot drift. Everything else in this module is a migration: a
//! table created by an earlier version keeps its columns, because
//! `CREATE TABLE IF NOT EXISTS` adds none.

use rusqlite::{Connection, Result};
use std::collections::HashSet;

/// `SCHEMA_VERSION`.
pub const SCHEMA_VERSION: i64 = 13;

pub const BENCHMARK_SYMBOL: &str = "SP500";

const SCHEMA_0: &str = include_str!("../sql/schema_0.sql");
const SCHEMA_1: &str = include_str!("../sql/schema_1.sql");

fn columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    let sql = format!("PRAGMA table_info({})", table);
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        out.insert(r.get::<_, String>(1)?);
    }
    Ok(out)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
        [table],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn add_missing(conn: &Connection, table: &str, cols: &[(&str, &str)]) -> Result<()> {
    let have = columns(conn, table)?;
    for (col, typ) in cols {
        if !have.contains(*col) {
            conn.execute_batch(&format!("ALTER TABLE {} ADD COLUMN {} {}", table, col, typ))?;
        }
    }
    Ok(())
}

/// `_init_schema`.
/// The tables the book took over (`docs/plans/stage-3c-switch.md` §8): what the
/// earlier app kept of the person's money. Once their rows are in the book they
/// are dropped from the live file (`drop_figure_tables`), the file snapshotted
/// first; an earlier file, or its snapshot, still holds them for the import and
/// the comparison to read.
pub const FIGURE_TABLES: [&str; 11] = [
    "activities", "securities", "accounts", "balances", "nav_history", "grouped_trades",
    "fx_rates", "benchmark_prices", "distributions", "distribution_fetches", "margin",
];

/// The meta key that says the figure tables were dropped from this file.
pub const FIGURES_MOVED_META: &str = "figure_tables_moved";

/// Whether this file's figure tables were dropped, the book holding their rows.
pub fn figures_moved(conn: &Connection) -> Result<bool> {
    if !table_exists(conn, "meta")? {
        return Ok(false);
    }
    let v: Option<String> = conn.query_row("SELECT value FROM meta WHERE key = ?", [FIGURES_MOVED_META], |r| r.get(0)).ok();
    Ok(v.is_some_and(|v| !v.is_empty()))
}

/// Drop the figure tables from this file, and note when.
pub fn drop_figure_tables(conn: &Connection, at: &str) -> Result<()> {
    crate::atomically(conn, || {
        for t in FIGURE_TABLES {
            conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{t}\""))?;
        }
        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![FIGURES_MOVED_META, at],
        )?;
        Ok(())
    })
}

/// The tables the book's orders and brackets took over (`docs/plans/stage-4-execution.md`):
/// dropped from the live file once carried into the book, the file snapshotted first.
pub const ORDER_TABLES: [&str; 2] = ["orders", "brackets"];

/// The meta key that says the order tables were carried into the book and dropped.
pub const ORDERS_MOVED_META: &str = "order_tables_moved";

/// Whether this file's orders and brackets were carried into the book.
pub fn orders_moved(conn: &Connection) -> Result<bool> {
    if !table_exists(conn, "meta")? {
        return Ok(false);
    }
    let v: Option<String> = conn.query_row("SELECT value FROM meta WHERE key = ?", [ORDERS_MOVED_META], |r| r.get(0)).ok();
    Ok(v.is_some_and(|v| !v.is_empty()))
}

/// Drop the order tables from this file, and note when.
pub fn drop_order_tables(conn: &Connection, at: &str) -> Result<()> {
    crate::atomically(conn, || {
        for t in ORDER_TABLES {
            conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{t}\""))?;
        }
        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![ORDERS_MOVED_META, at],
        )?;
        Ok(())
    })
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    crate::atomically(conn, || {
        conn.execute_batch(SCHEMA_0)?;

        migrate_nav_history(conn)?;
        ensure_bar_columns(conn)?;
        ensure_activity_security_id(conn)?;
        migrate_spy_meta(conn)?;
        ensure_quote_columns(conn)?;
        ensure_shorts_columns(conn)?;
        ensure_order_columns(conn)?;
        ensure_account_columns(conn)?;

        conn.execute_batch(SCHEMA_1)?;

        ensure_notifications_columns(conn)?;
        ensure_news_columns(conn)?;
        ensure_filings_columns(conn)?;
        migrate_history_sources(conn)?;

        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params!["schema_version", SCHEMA_VERSION.to_string()],
        )?;
        // last, so the triggers are made from the columns as the migrations left them
        crate::gens::install(conn)?;
        // a file whose figures the book holds keeps none of their tables
        if figures_moved(conn)? {
            for t in FIGURE_TABLES {
                conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{t}\""))?;
            }
        }
        // and none of the orders the book carries
        if orders_moved(conn)? {
            for t in ORDER_TABLES {
                conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{t}\""))?;
            }
        }
        Ok(())
    })
}

/// `_migrate_nav_history`: the table gained an account column and a
/// composite key, so a single-account history is moved under the empty id.
fn migrate_nav_history(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "nav_history")? {
        return Ok(());
    }
    let cols = columns(conn, "nav_history")?;
    if cols.contains("account_id") {
        return Ok(());
    }
    conn.execute_batch(
        "ALTER TABLE nav_history RENAME TO nav_history_old;
         CREATE TABLE nav_history (
            account_id TEXT NOT NULL DEFAULT '',
            date TEXT NOT NULL,
            equity REAL,
            currency TEXT,
            net_deposits REAL,
            PRIMARY KEY (account_id, date)
         );
         INSERT OR REPLACE INTO nav_history (account_id, date, equity, currency, net_deposits)
            SELECT '', date, equity, currency, net_deposits FROM nav_history_old;
         DROP TABLE nav_history_old;",
    )?;
    Ok(())
}

/// `_ensure_bar_columns`: bars stored before the chart drew candles hold
/// closes only, so the table is rebuilt and refetched rather than patched.
fn ensure_bar_columns(conn: &Connection) -> Result<()> {
    let cols = columns(conn, "price_bars")?;
    if !cols.is_empty() && !cols.contains("open") {
        conn.execute_batch(
            "DROP TABLE price_bars;
             DELETE FROM bar_fetches;
             CREATE TABLE price_bars (symbol TEXT NOT NULL, tf TEXT NOT NULL, ts INTEGER NOT NULL, open REAL, high REAL, low REAL, close REAL NOT NULL, volume REAL, source TEXT NOT NULL DEFAULT '', PRIMARY KEY (symbol, tf, ts));",
        )?;
    }
    Ok(())
}

/// `_ensure_activity_security_id`.
fn ensure_activity_security_id(conn: &Connection) -> Result<()> {
    add_missing(conn, "activities", &[("security_id", "TEXT")])
}

fn ensure_quote_columns(conn: &Connection) -> Result<()> {
    add_missing(
        conn,
        "quotes",
        &[("price_change", "REAL"), ("percent_change", "REAL"), ("prev_close", "REAL")],
    )
}

fn ensure_shorts_columns(conn: &Connection) -> Result<()> {
    add_missing(conn, "shorts", &[("read_version", "INTEGER"), ("name", "TEXT")])
}

fn ensure_order_columns(conn: &Connection) -> Result<()> {
    add_missing(
        conn,
        "orders",
        &[
            ("source", "TEXT"),
            ("ws_status", "TEXT"),
            ("filled_qty", "REAL"),
            ("avg_fill", "REAL"),
            ("submitted_at", "TEXT"),
            ("expires_at", "TEXT"),
            ("parent_id", "TEXT"),
            ("role", "TEXT"),
            ("fill_booked_qty", "REAL"),
        ],
    )?;
    add_missing(conn, "brackets", &[("seen_held", "INTEGER"), ("missed_at", "TEXT")])
}

fn ensure_account_columns(conn: &Connection) -> Result<()> {
    add_missing(conn, "accounts", &[("margin_account_id", "TEXT")])
}

fn ensure_notifications_columns(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "notifications")? {
        return Ok(());
    }
    add_missing(conn, "notifications", &[("read_at", "TEXT")])
}

/// `_ensure_news_columns`: a news table from before releases were told
/// apart gains the kind, and each row is told by the name of the wire it came
/// on -- a wire's item is a release, a publisher's a story.
fn ensure_news_columns(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "news")? {
        return Ok(());
    }
    // `summary` is what the source said beneath the headline; a row read before this is simply
    // without one
    add_missing(conn, "news", &[("kind", "TEXT"), ("summary", "TEXT")])?;
    conn.execute_batch(
        "UPDATE news SET kind = CASE WHEN LOWER(COALESCE(wire, '')) LIKE '%wire%' OR LOWER(COALESCE(wire, '')) LIKE '%newsfile%' OR LOWER(COALESCE(wire, '')) LIKE '%cision%' OR LOWER(COALESCE(wire, '')) LIKE '%cnw%' THEN 'release' ELSE 'story' END WHERE kind IS NULL OR kind = ''",
    )?;
    Ok(())
}

/// `_ensure_filings_columns`: a filings table created under the
/// single-source schema brought up to the multi-source shape. The old
/// `file`/`submitted`/`submitted_at` columns are left in place but unused; a
/// refresh repopulates every row under the new ones.
fn ensure_filings_columns(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "filings")? {
        return Ok(());
    }
    add_missing(
        conn,
        "filings",
        &[
            ("source", "TEXT"),
            ("category", "TEXT"),
            ("type", "TEXT"),
            ("title", "TEXT"),
            ("date", "TEXT"),
            ("date_text", "TEXT"),
            ("subject", "TEXT"),
            ("summary", "TEXT"),
            ("enriched_at", "TEXT"),
            ("enrich_version", "INTEGER"),
            ("enrich_final", "INTEGER"),
            ("enrich_reads", "INTEGER"),
        ],
    )
}

/// `_migrate_spy_meta`: one-shot copy of the legacy `meta.spy_by_date`
/// map into `benchmark_prices`.
fn migrate_spy_meta(conn: &Connection) -> Result<()> {
    let already: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM benchmark_prices WHERE symbol = ? LIMIT 1)",
        [BENCHMARK_SYMBOL],
        |r| r.get(0),
    )?;
    if already > 0 {
        return Ok(());
    }
    let raw: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'spy_by_date'", [], |r| r.get(0))
        .ok();
    let raw = match raw { Some(r) => r, None => return Ok(()) };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) { Ok(v) => v, Err(_) => return Ok(()) };
    let map = match parsed.as_object() { Some(m) => m, None => return Ok(()) };
    for (day, val) in map {
        if let Some(px) = val.as_f64() {
            conn.execute(
                "INSERT INTO benchmark_prices(symbol, date, close) VALUES (?, ?, ?) ON CONFLICT(symbol, date) DO UPDATE SET close = excluded.close",
                rusqlite::params![BENCHMARK_SYMBOL, day, px],
            )?;
        }
    }
    Ok(())
}

/// `_migrate_history_sources`: runs once when the chart's history
/// sources change. Bars from a source that gave closes only are dropped; the
/// fetch stamps of every symbol either source served are dropped, so the chart
/// refetches the whole span from the source that replaced it.
fn migrate_history_sources(conn: &Connection) -> Result<()> {
    let done: i64 = conn.query_row(
        "SELECT COUNT(*) FROM meta WHERE key = 'history_sources_migrated'",
        [],
        |r| r.get(0),
    )?;
    if done > 0 {
        return Ok(());
    }
    conn.execute_batch(
        "DELETE FROM price_history WHERE source = 'coingecko';
         DELETE FROM history_fetches WHERE symbol NOT IN (SELECT DISTINCT symbol FROM price_history WHERE source NOT IN ('coingecko', 'cboe_ca'));
         DELETE FROM price_bars WHERE source = 'coingecko';
         DELETE FROM bar_fetches WHERE symbol NOT IN (SELECT DISTINCT symbol FROM price_bars);",
    )?;
    // a stamp claiming a span its bars begin well after is dropped, so the
    // chain is asked again for the earlier days
    conn.execute_batch(
        "DELETE FROM history_fetches WHERE symbol IN (SELECT h.symbol FROM history_fetches h JOIN (SELECT symbol, MIN(date) AS first FROM price_history GROUP BY symbol) p ON p.symbol = h.symbol WHERE julianday(p.first) - julianday(h.start) > 7);
         DELETE FROM bar_fetches WHERE (symbol, tf) IN (SELECT b.symbol, b.tf FROM bar_fetches b JOIN (SELECT symbol, tf, MIN(ts) AS first FROM price_bars GROUP BY symbol, tf) p ON p.symbol = b.symbol AND p.tf = b.tf WHERE p.first - b.start_ts > 7 * 86400);
         INSERT INTO meta(key, value) VALUES ('history_sources_migrated', '1');",
    )?;
    Ok(())
}
