//! Migrations run by `ensure` on a database an earlier version wrote.

mod common;
use bagholder_store::{feeds, market, schema};
use common::*;
use serde_json::{json, Value};

#[test]
fn test_replaced_sources_are_refetched_once() {
    let d = db();
    let c = &d.conn;
    let stamp = "2026-09-07T00:00:00Z";
    market::upsert_price_history(c, "DOT", &[json!({"date": "2026-02-02", "open": null, "high": null, "low": null, "close": 9.5, "volume": null})], "coingecko").unwrap();
    market::mark_history_fetched(c, "DOT", "2026-02-02", stamp).unwrap();
    market::upsert_price_history(c, "MAXQ", &[json!({"date": "2026-06-09", "open": 0.4, "high": 0.4, "low": 0.4, "close": 0.4, "volume": 1})], "cboe_ca").unwrap();
    market::mark_history_fetched(c, "MAXQ", "2025-10-14", stamp).unwrap();
    market::upsert_price_history(c, "RDDY", &[json!({"date": "2026-02-02", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1})], "tmx").unwrap();
    market::mark_history_fetched(c, "RDDY", "2026-02-02", stamp).unwrap();
    market::upsert_price_history(c, "USDC", &[json!({"date": "2026-02-25", "open": 1, "high": 1, "low": 1, "close": 1, "volume": 1})], "coinbase").unwrap();
    market::mark_history_fetched(c, "USDC", "2026-01-10", stamp).unwrap(); // claimed January, has late February
    market::upsert_price_bars(c, "USDC", "1h", &[json!({"time": 1772000000, "open": 1, "high": 1, "low": 1, "close": 1, "volume": 0})], "coinbase").unwrap();
    market::mark_bars_fetched(c, "USDC", "1h", 1768000000, stamp).unwrap(); // claimed from 2026-01-10
    market::upsert_price_bars(c, "RDDY", "1h", &[json!({"time": 1768003200, "open": 1, "high": 1, "low": 1, "close": 1, "volume": 0})], "tmx").unwrap();
    market::mark_bars_fetched(c, "RDDY", "1h", 1768000000, stamp).unwrap();
    c.execute("DELETE FROM meta WHERE key = 'history_sources_migrated'", []).unwrap();
    schema::init_schema(c).unwrap();
    let hist = |s: &str| market::price_history(c, s, "", "").unwrap();
    assert_eq!(hist("DOT"), Vec::<Value>::new(), "close-only bars are gone");
    assert_eq!(market::history_fetch(c, "DOT").unwrap(), Value::Null, "and their fetch stamp, so the chart refetches");
    assert_eq!(hist("MAXQ").len(), 1, "Cboe's real bars stay");
    assert_eq!(market::history_fetch(c, "MAXQ").unwrap(), Value::Null, "but the span is refetched from TMX, which reaches further back");
    assert_eq!(hist("RDDY").len(), 1, "TMX candles stay");
    assert_ne!(market::history_fetch(c, "RDDY").unwrap(), Value::Null);
    assert_eq!(hist("USDC").len(), 1, "real bars stay");
    assert_eq!(market::history_fetch(c, "USDC").unwrap(), Value::Null, "a stamp claiming days its bars do not reach is dropped");
    assert_eq!(market::bar_fetch(c, "USDC", "1h").unwrap(), Value::Null, "the same for intraday stamps");
    assert_ne!(market::bar_fetch(c, "RDDY", "1h").unwrap(), Value::Null, "an honest intraday stamp stays");
    // runs once: rows added afterwards under an old source name are left alone
    market::upsert_price_history(c, "DOT", &[json!({"date": "2026-02-03", "open": null, "high": null, "low": null, "close": 9.6, "volume": null})], "coingecko").unwrap();
    schema::init_schema(c).unwrap();
    assert_eq!(hist("DOT").len(), 1);
}

/// ShortsColumnTest.
#[test]
fn test_a_table_from_the_version_before_gains_the_column_and_keeps_its_rows() {
    let d = bare();
    d.conn.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
         INSERT INTO meta VALUES ('schema_version', '12');
         CREATE TABLE shorts (symbol TEXT NOT NULL, exchange TEXT NOT NULL DEFAULT '', market TEXT, as_of TEXT,
          shares REAL, previous REAL, previous_of TEXT, change REAL, float_shares REAL, of_float REAL,
          average_volume REAL, days_to_cover REAL, volume_of TEXT, volume_span TEXT, short_volume REAL,
          total_volume REAL, volume_pct REAL, series TEXT, fetched_at TEXT, PRIMARY KEY (symbol, exchange));
         INSERT INTO shorts (symbol, exchange, shares, fetched_at) VALUES ('QNC','TSX-V',2667164,'2026-09-15T20:00:00Z');",
    ).unwrap();
    d.ensure();
    let row = feeds::shorts_for(&d.conn, "QNC", "TSX-V").unwrap().unwrap();
    assert_eq!(f(&row["shares"]), 2667164.0);
    assert_eq!(row["readVersion"], json!(0)); // unmarked, so it is read again once
}
