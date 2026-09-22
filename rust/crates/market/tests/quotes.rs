//! Where a listing's quote and history are read from.
use bagholder_market::{history, quotes};
use bagholder_model::input::Listing;

#[test]
fn test_quote_sources_cover_every_held_kind() {
    let src = |sym: &str, ex: &str, ccy: &str| quotes::quote_source(&Listing::new(sym, ex, ccy, "Shares"));
    assert_eq!(src("LUNR", "NASDAQ", "USD"), Some(("yahoo_quote".to_string(), "LUNR".to_string())),
               "a US listing is quoted where its quote is live: TMX stamps one fifteen minutes behind");
    // a watched listing keeps no currency: the venue names the market, never a Toronto form of a Nasdaq ticker
    for sym in ["PLTR", "LUNR", "ASTS"] {
        assert_eq!(src(sym, "NASDAQ", ""), Some(("yahoo_quote".to_string(), sym.to_string())), "{}.TO is another security, not the Nasdaq listing", sym);
    }
    assert_eq!(quotes::yahoo_forms(&Listing::new("PLTR", "NASDAQ", "", "")), ["PLTR"]);
    assert_eq!(quotes::yahoo_forms(&Listing::new("HHIS.U", "TSX", "USD", ""))[0], "HHIS-U.TO", "a Toronto listing in US dollars is still Toronto's");
    assert_eq!(quotes::yahoo_forms(&Listing::new("QNC", "", "", ""))[0], "QNC.TO", "no venue and no currency: Canada, as before");
    assert_eq!(quotes::yahoo_forms(&Listing::new("ASTS", "", "USD", "")), ["ASTS"]);
}

#[test]
fn test_history_parsers_and_sources() {
    assert_eq!(history::history_candidates(&Listing::new("PLTR", "NASDAQ", "", "Shares")),
               vec![("tmx".to_string(), "PLTR:US".to_string()), ("yahoo".to_string(), "PLTR".to_string())],
               "a watched US listing's chart falls back to the Nasdaq listing's bars, not a Toronto security's");
}

// --- when a price can have moved ----------------------------------------------

/// Unix seconds for a New York wall-clock moment in September 2026 (EDT, UTC-4).
fn ny(day: u32, hour: u32, minute: u32) -> f64 {
    // 2026-09-01 00:00 EDT = 2026-09-01 04:00 UTC
    let sep1_utc_midnight = 1_788_220_800.0; // 2026-09-01T00:00:00Z
    sep1_utc_midnight + ((day - 1) as f64) * 86400.0 + ((hour + 4) as f64) * 3600.0 + (minute as f64) * 60.0
}

#[test]
fn test_the_markets_are_open_on_a_weekday_between_the_bell_and_the_settled_close() {
    use bagholder_market::quotes::markets_open;
    // 2026-09-21 is a Monday, 2026-09-19 a Saturday
    assert!(!markets_open(ny(21, 9, 29)));
    assert!(markets_open(ny(21, 9, 30)));
    assert!(markets_open(ny(21, 16, 10)), "the closing print is still settling");
    assert!(!markets_open(ny(21, 16, 20)));
    assert!(!markets_open(ny(19, 11, 0)), "a Saturday");
    assert!(!markets_open(ny(20, 11, 0)), "a Sunday");
}

#[test]
fn test_a_share_read_after_the_close_is_not_read_again_until_the_open_and_a_coin_always_is() {
    use bagholder_market::quotes::can_have_moved;
    let friday_after_close = ny(18, 16, 45);
    let saturday_night = ny(19, 23, 0);
    let sunday = ny(20, 14, 0);
    let monday_open = ny(21, 9, 31);
    // read once after Friday's close: nothing to ask all weekend
    assert!(!can_have_moved("tmx", Some(friday_after_close), saturday_night));
    assert!(!can_have_moved("yahoo", Some(friday_after_close), sunday));
    // but the moment the market trades again, it can
    assert!(can_have_moved("tmx", Some(friday_after_close), monday_open));
    // a quote last read during Friday's session missed the close: it is read once more
    assert!(can_have_moved("tmx", Some(ny(18, 15, 0)), saturday_night));
    // never read at all: read
    assert!(can_have_moved("tmx", None, saturday_night));
    // a coin trades at three on a Sunday morning
    assert!(can_have_moved("coinbase", Some(sunday - 120.0), sunday));
}

// --- the archive ------------------------------------------------------------------

/// A listing its source has no bars for is asked once per top-up, like any other:
/// it used to be asked again on every pass, without end, because only a read that
/// found bars was ever stamped.
#[test]
fn test_a_listing_with_no_bars_is_not_asked_again_until_the_next_top_up() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&conn).unwrap();
    let recs = [Listing::new("QMET", "TSXV", "CAD", "Shares")];
    let now = ny(16, 12, 0);
    let due = |at: f64| history::archive_intraday_due(&conn, &recs, "2026-09-16", at).iter().map(|t| (t.0, t.1.clone())).collect::<Vec<_>>();
    assert_eq!(due(now), vec![(0, "QMET".to_string())], "never asked: first in line");
    assert_eq!(history::archive_next_due_secs(&conn, &recs, "2026-09-16", now), Some(0.0));
    history::record_intraday_miss(&conn, "QMET", "1h", "2026-09-16T16:00:00Z"); // asked, and the source had nothing
    assert_eq!(due(now), vec![], "a miss is a read");
    assert_eq!(history::archive_next_due_secs(&conn, &recs, "2026-09-16", now), Some(history::ARCHIVE_TOPUP_HOURS * 3600.0));
    assert_eq!(due(now + 21.0 * 3600.0), vec![(1, "QMET".to_string())], "due again with the others' top-up");
    assert_eq!(history::archive_next_due_secs(&conn, &[], "2026-09-16", now), None, "nothing archived: only the book can make work");
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
