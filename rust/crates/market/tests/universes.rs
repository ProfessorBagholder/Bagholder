//! Market universes: parsing; storage is tested with the store crate.
use bagholder_market::universes;
use serde_json::{json, Value};

fn rows_data() -> Value {
    json!({"data": {"rows": [
        {"symbol": "NVDA", "name": "NVIDIA Corporation Common Stock", "lastsale": "$219.41", "pctchange": "0.808%", "marketCap": "5350000000000.00", "sector": "Technology", "country": "United States"},
        {"symbol": "JPM", "name": "JP Morgan", "lastsale": "$210.00", "pctchange": "-0.5%", "marketCap": "600000000000.00", "sector": "Finance", "country": "United States"},
        {"symbol": "TSM", "name": "Taiwan Semiconductor", "lastsale": "$250.00", "pctchange": "1.2%", "marketCap": "1300000000000.00", "sector": "Technology", "country": "Taiwan"},
        {"symbol": "SHOP", "name": "Shopify", "lastsale": "$130.00", "pctchange": "3.3%", "marketCap": "170000000000.00", "sector": "Technology", "country": "Canada"},
        {"symbol": "XYZ", "name": "No cap", "lastsale": "$1.00", "pctchange": "N/A", "marketCap": "", "sector": "Miscellaneous", "country": "United States"},
        {"symbol": "ABC", "name": "No country", "lastsale": "$2.00", "pctchange": "0.1%", "marketCap": "100.00", "sector": "Telecommunications", "country": ""},
    ]}})
}
fn syms(rows: &[bagholder_model::input::UniverseRow]) -> Vec<&str> { rows.iter().map(|r| r.symbol.as_str()).collect() }

#[test]
fn test_rows_are_parsed_and_sectors_folded() {
    let rows = universes::parse_screener(&rows_data());
    let first: Vec<(&str, Option<f64>, Option<f64>, f64, &str, &str)> = rows[..2].iter().map(|r| (r.symbol.as_str(), r.last, r.percent_change, r.cap, r.sector.as_str(), r.country.as_str())).collect();
    assert_eq!(first, vec![
        ("NVDA", Some(219.41), Some(0.808), 5.35e12, "Information Technology", "United States"),
        ("JPM", Some(210.0), Some(-0.5), 6e11, "Financials", "United States"),
    ]);
    let sectors: Vec<&str> = rows[4..].iter().map(|r| r.sector.as_str()).collect();
    assert_eq!(sectors, ["Not classified", "Communication Services"]);
    assert_eq!(rows[4].percent_change, None);
}

#[test]
fn test_us_and_international_are_the_largest_by_cap() {
    let rows = universes::parse_screener(&rows_data());
    assert_eq!(syms(&universes::us_rows(&rows, 5)), ["NVDA", "JPM"]);
    assert_eq!(syms(&universes::intl_rows(&rows, 5)), ["TSM"]);
    assert_eq!(universes::us_rows(&rows, 1)[0].value, 5.35e12);
}

#[test]
fn test_constituents_and_tile_quote() {
    let cons = universes::parse_constituents(&json!({"data": {"constituents": [{"symbol": "RY", "quotedMarketValue": 398317400940u64, "longName": "Royal Bank of Canada", "weight": 9.823, "exchange": "TSX"}, {"weight": 1}]}}));
    assert_eq!(cons, vec![universes::Constituent { symbol: "RY".into(), name: "Royal Bank of Canada".into(), weight: 9.823, cap: 398317400940.0, exchange: "TSX".into() }]);
    let q = universes::parse_tile_quote(&json!({"data": {"getQuoteBySymbol": {"symbol": "RY", "name": "Royal Bank", "price": 180.1, "percentChange": 0.42, "sector": "Financial Services"}}}));
    assert_eq!(q, Some(universes::TileQuote { percent_change: Some(0.42), sector: "Financials".into(), name: "Royal Bank".into() }));
    assert_eq!(universes::parse_tile_quote(&json!({"data": {"getQuoteBySymbol": null}})), None);
}
