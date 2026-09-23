//! Indices, futures, rates and currency pairs the watchlist can follow, and
//! how the model keys and prices them.

use serde_json::{json, Value};

use bagholder_model::base::build_base;
use bagholder_model::instruments::{find, implied_rate, kind_label, label, search};
use bagholder_model::markets::{tile_rows, watch_quote_key, watch_rows, watch_symbols};

fn syms(text: &str, k: usize) -> Vec<String> {
    search(text).iter().take(k).map(|r| r.symbol.clone()).collect()
}

fn v(s: &[&str]) -> Vec<String> {
    s.iter().map(|x| x.to_string()).collect()
}

fn base_with(snapshot: Value, quotes: Value) -> bagholder_model::base::Base {
    build_base(&snapshot, &json!({"fx": {}, "benchmark": {}, "quotes": quotes}), &Default::default(), Some("2026-09-16"))
}

fn sent<T: serde::Serialize>(rows: &[T]) -> Vec<serde_json::Value> {
    rows.iter().map(|r| serde_json::to_value(r).unwrap()).collect()
}

#[test]
fn test_aliases_find_what_people_type() {
    assert_eq!(syms("WTI", 1), v(&["CL"]));
    assert_eq!(syms("crude", 2), v(&["CL", "BZ"]));
    assert_eq!(syms("NDX", 1), v(&["NDX"]));
    assert_eq!(syms("nasdaq 100", 1), v(&["NDX"]));
    assert_eq!(syms("VIX", 1), v(&["VIX"]));
    assert_eq!(syms("volatility", 1), v(&["VIX"]));
    assert_eq!(syms("gold", 1), v(&["GC"]));
    assert_eq!(syms("ES", 1), v(&["ES"]), "the S&P 500 E-mini, the after-hours read");
    assert_eq!(syms("futures", 4), v(&["ES", "NQ", "YM", "RTY"]));
    assert_eq!(syms("dow futures", 1), v(&["YM"]));
    assert_eq!(syms("nasdaq futures", 1), v(&["NQ"]));
    assert_eq!((find("es", "cme").unwrap().yahoo, kind_label("Future").as_str()), ("ES=F", "Futures"));
    assert!(search("ZZZZ").is_empty());
    assert!(search("V").is_empty(), "a single letter is not a search for every V");
    assert_eq!(syms("VI", 1), v(&["VIX"]));
    let row = &search("VIX")[0];
    assert_eq!((row.name.as_str(), row.exchange.as_str(), row.currency.as_str(), row.kind.as_deref()), ("CBOE Volatility Index", "Index", "USD", Some("Index")));
}

#[test]
fn test_find_by_symbol_and_venue() {
    assert_eq!(find("cl", "nymex").unwrap().yahoo, "CL=F");
    assert!(find("CL", "TSX").is_none(), "a listing with the same letters is not the future");
    assert!(find("SHOP", "TSX").is_none());
}

#[test]
fn test_an_instrument_takes_the_directory_name_and_a_kind() {
    let base = base_with(json!({"watchlist": [{"symbol": "CL", "exchange": "NYMEX", "name": "Crude Oil (WTI)", "currency": "USD"}]}), json!({"CL@NYMEX": {"price": 99.4, "priceChange": 6.37, "percentChange": 6.85}}));
    assert_eq!(sent(&watch_symbols(&base)), vec![json!({"symbol": "CL", "exchange": "NYMEX", "currency": "USD", "kind": "Instrument", "quoteKey": "CL@NYMEX", "yahoo": "CL=F"})]);
    let rows = sent(&watch_rows(&base, &[]));
    assert_eq!((&rows[0]["sector"], &rows[0]["kind"], &rows[0]["last"]), (&json!("Commodities"), &json!("Commodity"), &json!(99.4)), "an instrument groups under its kind on the heatmap");
}

#[test]
fn test_a_coin_from_the_book_is_quoted_by_coinbase() {
    let base = base_with(json!({"watchlist": [{"symbol": "BTC", "exchange": "CRYPTO", "name": "Bitcoin", "currency": "CAD"}]}), json!({"BTC@CRYPTO": {"price": 150000.0}}));
    assert_eq!(sent(&watch_symbols(&base)), vec![json!({"symbol": "BTC", "exchange": "CRYPTO", "currency": "USD", "kind": "Crypto", "quoteKey": "BTC@CRYPTO"})], "the USD pair, whatever currency the book holds the coin in");
    let row = &sent(&watch_rows(&base, &[]))[0];
    assert_eq!((&row["sector"], &row["kind"], &row["last"]), (&json!("Digital assets"), &json!("Crypto"), &json!(150000.0)));
}

#[test]
fn test_the_directory_finds_them_by_the_words_people_type() {
    assert_eq!(syms("fed", 99), v(&["ZQ"]));
    assert_eq!(syms("sofr", 99), v(&["SR3"]));
    assert_eq!(syms("fed funds", 99), v(&["ZQ"]));
    assert_eq!(label("ZQ"), "FED FUNDS");
}

#[test]
fn test_the_rate_is_the_price_taken_from_a_hundred_and_nothing_else_carries_one() {
    assert_eq!(implied_rate("ZQ", Some(96.13)), Some(3.87));
    assert_eq!(implied_rate("SR3", Some(95.765)), Some(4.235));
    assert_eq!(implied_rate("ES", Some(7674.0)), None, "an index future prices no rate");
    assert_eq!(implied_rate("ZQ", None), None, "and an unquoted contract prices none either");
}

#[test]
fn test_a_tile_carries_the_rate_beside_the_published_price_and_the_day_runs_the_other_way() {
    let mut quotes = serde_json::Map::new();
    quotes.insert(watch_quote_key("ZQ", "CBOT"), json!({"price": 96.13, "priceChange": -0.157, "percentChange": -0.163}));
    quotes.insert(watch_quote_key("ES", "CME"), json!({"price": 7674.0, "priceChange": 18.0, "percentChange": 0.24}));
    let base = base_with(json!({"tiles": [{"symbol": "ZQ", "exchange": "CBOT"}, {"symbol": "ES", "exchange": "CME"}]}), Value::Object(quotes));
    let rows = sent(&tile_rows(&base));
    let zq = rows.iter().find(|r| r["symbol"] == "ZQ").unwrap();
    let es = rows.iter().find(|r| r["symbol"] == "ES").unwrap();
    assert_eq!((&zq["last"], &zq["rate"], &zq["rateChange"]), (&json!(96.13), &json!(3.87), &json!(0.157)),
        "the price as published, the rate it prices, and a day that cut the price raised the rate");
    assert!(es.get("rate").is_none(), "nothing else carries one");
}
