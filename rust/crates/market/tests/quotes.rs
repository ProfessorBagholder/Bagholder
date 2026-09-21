//! Where a listing's quote and history are read from.
use bagholder_market::{history, quotes};
use serde_json::json;

#[test]
fn test_quote_sources_cover_every_held_kind() {
    let src = |sym: &str, ex: &str, ccy: &str| quotes::quote_source(&json!({"symbol": sym, "exchange": ex, "currency": ccy, "kind": "Shares"}));
    assert_eq!(src("LUNR", "NASDAQ", "USD"), Some(("yahoo_quote".to_string(), "LUNR".to_string())),
               "a US listing is quoted where its quote is live: TMX stamps one fifteen minutes behind");
    // a watched listing keeps no currency: the venue names the market, never a Toronto form of a Nasdaq ticker
    for sym in ["PLTR", "LUNR", "ASTS"] {
        assert_eq!(src(sym, "NASDAQ", ""), Some(("yahoo_quote".to_string(), sym.to_string())), "{}.TO is another security, not the Nasdaq listing", sym);
    }
    assert_eq!(quotes::yahoo_forms(&json!({"symbol": "PLTR", "exchange": "NASDAQ", "currency": ""})), ["PLTR"]);
    assert_eq!(quotes::yahoo_forms(&json!({"symbol": "HHIS.U", "exchange": "TSX", "currency": "USD"}))[0], "HHIS-U.TO", "a Toronto listing in US dollars is still Toronto's");
    assert_eq!(quotes::yahoo_forms(&json!({"symbol": "QNC", "exchange": "", "currency": ""}))[0], "QNC.TO", "no venue and no currency: Canada, as before");
    assert_eq!(quotes::yahoo_forms(&json!({"symbol": "ASTS", "exchange": "", "currency": "USD"})), ["ASTS"]);
}

#[test]
fn test_history_parsers_and_sources() {
    assert_eq!(history::history_candidates(&json!({"symbol": "PLTR", "exchange": "NASDAQ", "currency": "", "kind": "Shares"})),
               vec![("tmx".to_string(), "PLTR:US".to_string()), ("yahoo".to_string(), "PLTR".to_string())],
               "a watched US listing's chart falls back to the Nasdaq listing's bars, not a Toronto security's");
}

/// A listing its source has no bars for is asked once per top-up, like any other:
/// it used to be asked again on every pass, without end, because only a read that
/// found bars was ever stamped.
#[test]
fn test_a_listing_with_no_bars_is_not_asked_again_until_the_next_top_up() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&conn).unwrap();
    let recs = [json!({"symbol": "QMET", "exchange": "TSXV", "currency": "CAD", "kind": "Shares"})];
    let now = 1_789_574_400.0; // 2026-09-16T16:00:00Z
    let due = |at: f64| history::archive_intraday_due(&conn, &recs, "2026-09-16", at).iter().map(|t| (t.0, t.1.clone())).collect::<Vec<_>>();
    assert_eq!(due(now), vec![(0, "QMET".to_string())], "never asked: first in line");
    history::record_intraday_miss(&conn, "QMET", "1h", "2026-09-16T16:00:00Z"); // asked, and the source had nothing
    assert_eq!(due(now), vec![], "a miss is a read");
    assert_eq!(due(now + 21.0 * 3600.0), vec![(1, "QMET".to_string())], "due again with the others' top-up");
}
