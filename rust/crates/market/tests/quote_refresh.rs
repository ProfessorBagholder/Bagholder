//! The venue-keyed quote refresh.

use bagholder_model::input::Listing;

#[test]
fn test_quote_refresh_keys_a_watched_listing_by_venue() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&conn).unwrap();
    let needing = bagholder_market::quotes::quote_symbols_needing_refresh(
        &conn,
        &[
            Listing::new("AAPL", "NEO", "CAD", "Shares"),
            Listing { quote_key: Some("AAPL@NASDAQ".into()), ..Listing::new("AAPL", "NASDAQ", "USD", "Shares") },
        ],
        1_800_000_000.0,
        bagholder_market::quotes::QUOTE_REFRESH_MINUTES,
    )
    .unwrap();
    let got: Vec<(String, String)> = needing.into_iter().map(|(k, s, _)| (k, s)).collect();
    assert_eq!(
        got,
        vec![("AAPL".to_string(), "cboe_ca".to_string()), ("AAPL@NASDAQ".to_string(), "yahoo_quote".to_string())],
        "the held CDR and the watched US listing keep separate quotes, each from a feed live for its market"
    );
}
