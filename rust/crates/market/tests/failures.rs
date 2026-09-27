//! Failures the market readers used to drop: a store that refuses a write or a
//! read, a source that does not answer. Each is returned to the caller, which
//! says it (the server puts it in the header's error line), never read as
//! nothing. Nothing here reaches the network: the process is offline, so every
//! source is refused before a request leaves.

use std::collections::HashMap;

use bagholder_market::{history, refresh, shorts};
use bagholder_model::input::Listing;
use bagholder_market::shorts::{CaPositionRow, CaVolumeRow};

fn offline() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| std::env::set_var("BAGHOLDER_OFFLINE", "1"));
}

fn db() -> rusqlite::Connection {
    offline();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&conn).unwrap();
    conn
}

/// A store that refuses to remember a bar read: the misses are the only thing an
/// offline read writes.
fn refuse_misses(conn: &rusqlite::Connection) {
    conn.execute_batch("CREATE TRIGGER refuse_misses BEFORE INSERT ON meta WHEN NEW.key LIKE 'bars_miss:%' BEGIN SELECT RAISE(ABORT, 'refused'); END;").unwrap();
}

fn allow_misses(conn: &rusqlite::Connection) {
    conn.execute_batch("DROP TRIGGER refuse_misses;").unwrap();
}

#[test]
fn an_archive_pass_whose_bars_the_store_refuses_fails_and_the_next_one_goes_through() {
    let conn = db();
    let (today, now, stamp) = bagholder_market::clock_now();
    let recs = [Listing::new("QMET", "TSXV", "CAD", "Shares")];
    refuse_misses(&conn);
    let err = history::archive_intraday(&conn, &recs, &today, now, &stamp, 1).unwrap_err();
    assert!(err.to_string().contains("refused"), "{err}");
    allow_misses(&conn);
    assert_eq!(history::archive_intraday(&conn, &recs, &today, now, &stamp, 1).unwrap(), vec!["QMET".to_string()]);
}

#[test]
fn a_daily_bar_read_the_store_refuses_is_the_reads_failure() {
    let conn = db();
    let (today, now, stamp) = bagholder_market::clock_now();
    let rec = Listing::new("QMET", "TSXV", "CAD", "Shares");
    refuse_misses(&conn);
    let err = history::ensure_history(&conn, &rec, &today, &today, &today, now, &stamp).unwrap_err();
    assert!(err.to_string().contains("refused"), "{err}");
    allow_misses(&conn);
    assert_eq!(history::ensure_history(&conn, &rec, &today, &today, &today, now, &stamp).unwrap(), vec![]);
}

/// A store that cannot be read is not a listing with nothing stored.
#[test]
fn a_bar_store_that_cannot_be_read_is_not_a_chart_with_nothing_due() {
    let conn = db();
    let (today, now, _) = bagholder_market::clock_now();
    let rec = Listing::new("QMET", "TSXV", "CAD", "Shares");
    conn.execute_batch("DROP TABLE meta;").unwrap();
    assert!(history::daily_due(&conn, &rec, &today, &today, &today, now).is_err());
    assert!(history::offered_timeframes(&conn, &rec, &today, &today, now).is_err());
    assert!(history::intraday_ready(&conn, &rec, "1h", &today, &today, now).is_err());
}

/// The earlier store's series top-up: each part that failed is returned, the
/// others still run.
#[test]
fn a_market_series_top_up_returns_what_failed() {
    let conn = db();
    let err = refresh::refresh_all(&conn, &[]).unwrap_err();
    for source in ["Bank of Canada", "FRED", "Stooq", "TMX Money"] {
        assert!(err.contains(source), "{source} is not said in: {err}");
    }
    conn.execute_batch("DROP TABLE fx_rates;").unwrap();
    let err = refresh::refresh_fx(&conn).unwrap_err();
    assert!(err.contains("could not be stored"), "{err}");
    assert!(refresh::series_stale(&conn, "2026-09-16").is_err(), "a store that cannot be read is not a stale series");
}

/// Short interest from a regulator that did not answer is that regulator's
/// failure, never a record with no position.
#[test]
fn short_interest_from_a_regulator_that_did_not_answer_is_the_failure() {
    let conn = db();
    let err = shorts::for_listing(&conn, "GME", "NYSE", "USD", "2026-09-16", false, "").unwrap_err();
    assert!(err.contains("FINRA"), "{err}");
}

/// The market's trading days are read from the store: a store that cannot be
/// read fails the listing's read rather than leaving its days to cover unsaid.
#[test]
fn short_interest_whose_store_cannot_be_read_is_the_failure() {
    let conn = db();
    let mut position = HashMap::new();
    position.insert("QNC".to_string(), CaPositionRow { venue: "TSXV".into(), shares: 2667164.0, change: Some(64077.0), name: "QUANTUM EMOTION CORP.".into() });
    shorts::clear_files();
    shorts::ca_position_with("QNC", "TSX-V", "2026-09-16", || Ok(Some(("2026-09-15".to_string(), position)))).unwrap();
    let mut volume = HashMap::new();
    volume.insert("QNC".to_string(), CaVolumeRow { venue: "TSXV".into(), short_volume: 1_000_000.0, volume_pct: Some(20.0), total_volume: Some(5_000_000.0) });
    shorts::warm_ca_volume("ca_volume", "2026-09-01/2026-09-15", volume);
    assert!(shorts::for_listing(&conn, "QNC", "TSX-V", "CAD", "2026-09-16", false, "").unwrap().is_some());
    conn.execute_batch("DROP TABLE benchmark_prices;").unwrap();
    let err = shorts::for_listing(&conn, "QNC", "TSX-V", "CAD", "2026-09-16", false, "").unwrap_err();
    assert!(err.contains("could not be read"), "{err}");
}

/// A form TMX was not asked about, the process being offline, is no miss: the
/// failure is returned and nothing is remembered for the day.
#[test]
fn a_tmx_form_lookup_that_failed_is_not_remembered_as_a_miss() {
    let conn = db();
    assert!(bagholder_market::tmx::tmx_resolve(&conn, "QNC", "2026-09-16").is_err());
    assert_eq!(bagholder_store::tables::get_meta(&conn, "tmx_form:QNC", "").unwrap(), "");
}

/// The same for Coinbase's market for a pair.
#[test]
fn a_coinbase_market_lookup_that_failed_is_not_remembered_as_a_miss() {
    let conn = db();
    assert!(history::coinbase_market(&conn, "BTC-CAD", "2026-09-16").is_err());
    assert_eq!(bagholder_store::tables::get_meta(&conn, "coinbase_product:BTC-CAD", "").unwrap(), "");
}
