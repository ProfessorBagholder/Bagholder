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
