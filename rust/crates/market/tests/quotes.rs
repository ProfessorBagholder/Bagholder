//! Where a listing's quote and history are read from.
use bagholder_market::{history, quotes};
use bagholder_model::input::Listing;

#[test]
fn test_a_listing_s_yahoo_forms_are_its_own_venue_s() {
    assert_eq!(quotes::yahoo_forms(&Listing::new("PLTR", "NASDAQ", "", "")), ["PLTR"]);
    assert_eq!(quotes::yahoo_forms(&Listing::new("HHIS.U", "TSX", "USD", ""))[0], "HHIS-U.TO", "a Toronto listing in US dollars is still Toronto's");
    // no venue: no form, whatever the currency, since a currency names no venue
    for currency in ["", "CAD", "USD"] {
        assert!(quotes::yahoo_forms(&Listing::new("QNC", "", currency, "")).is_empty(), "{currency}");
    }
}

#[test]
fn test_history_parsers_and_sources() {
    assert_eq!(history::history_candidates(&Listing::new("PLTR", "NASDAQ", "", "Shares")),
               vec![("tmx".to_string(), "PLTR:US".to_string()), ("yahoo".to_string(), "PLTR".to_string())],
               "a watched US listing's chart falls back to the Nasdaq listing's bars, not a Toronto security's");
}

/// Unix seconds for a New York wall-clock moment in September 2026 (EDT, UTC-4).
fn ny(day: u32, hour: u32, minute: u32) -> f64 {
    // 2026-09-01 00:00 EDT = 2026-09-01 04:00 UTC
    let sep1_utc_midnight = 1_788_220_800.0; // 2026-09-01T00:00:00Z
    sep1_utc_midnight + ((day - 1) as f64) * 86400.0 + ((hour + 4) as f64) * 3600.0 + (minute as f64) * 60.0
}

// --- the archive ------------------------------------------------------------------

/// A listing its source has no bars for is asked once per top-up, like any other:
/// it used to be asked again on every pass, without end, because only a read that
/// found bars was ever stamped.
#[test]
fn test_a_listing_with_no_bars_is_not_asked_again_until_the_next_top_up() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    for m in &bagholder_sources::cache::MIGRATIONS {
        conn.execute_batch(m.sql).unwrap();
    }
    let recs = [Listing::new("QMET", "TSXV", "CAD", "Shares").keyed().unwrap()];
    let key = recs[0].key.clone();
    let now = ny(16, 12, 0);
    let due = |at: f64| history::archive_intraday_due(&conn, &recs, "2026-09-16", at).unwrap().iter().map(|t| (t.0, t.1.clone())).collect::<Vec<_>>();
    assert_eq!(due(now), vec![(0, key.clone())], "never asked: first in line");
    assert_eq!(history::archive_next_due_secs(&conn, &recs, "2026-09-16", now).unwrap(), Some(0.0));
    history::record_intraday_miss(&conn, &key, "1h", "2026-09-16T16:00:00Z").unwrap(); // asked, and the source had nothing
    assert_eq!(due(now), vec![], "a miss is a read");
    assert_eq!(history::archive_next_due_secs(&conn, &recs, "2026-09-16", now).unwrap(), Some(history::ARCHIVE_TOPUP_HOURS * 3600.0));
    assert_eq!(due(now + 21.0 * 3600.0), vec![(1, key)], "due again with the others' top-up");
    assert_eq!(history::archive_next_due_secs(&conn, &[], "2026-09-16", now).unwrap(), None, "nothing archived: only the book can make work");
}

/// An option contract's chart is its underlying's, in the contract's currency or
/// US dollars when the contract names none; any other listing charts as itself.
#[test]
fn test_an_option_charts_as_its_underlying() {
    let option = Listing::new("AAPL 261218C00200000", "", "", "Options");
    assert_eq!(history::chart_instrument(&option), Listing::new("AAPL", "", "USD", "Shares"));
    let in_cad = Listing::new("AAPL 261218C00200000", "NEO", "CAD", "Options");
    assert_eq!(history::chart_instrument(&in_cad), Listing::new("AAPL", "NEO", "CAD", "Shares"));
    let shares = Listing { start: Some("2026-01-02".into()), ..Listing::new("SHOP", "TSX", "CAD", "Shares") };
    assert_eq!(history::chart_instrument(&shares), shares, "a listing that is not a contract keeps every field");
}
