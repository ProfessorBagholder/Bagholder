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

/// One `getQuoteForSymbols` answer, made up in the shape TMX answers it (checked
/// against TMX 2026-11-02 with three constituents), cut to the fields read.
fn quotes_answer() -> Value {
    json!({"data": {"getQuoteForSymbols": [
        {"symbol": "AAA", "longname": "A Corp", "price": 10.5, "percentChange": 1.25},
        {"symbol": "BBB", "longname": "B Corp", "price": 20.0, "percentChange": -0.5},
        {"symbol": "CCC", "longname": "C Corp", "price": 30.0, "percentChange": null}
    ]}})
}

fn constituents() -> Vec<universes::Constituent> {
    universes::parse_constituents(&json!({"data": {"constituents": [
        {"symbol": "AAA", "quotedMarketValue": 100, "longName": "A Corp", "weight": 5.0, "exchange": "TSX"},
        {"symbol": "BBB", "quotedMarketValue": 200, "longName": "B Corp", "weight": 3.0, "exchange": "TSX"},
        {"symbol": "CCC", "quotedMarketValue": 300, "longName": "C Corp", "weight": 2.0, "exchange": "TSX"}
    ]}}))
}

/// Every constituent's day comes from the one answer; a sector is asked only
/// where none is known, or the one known is older than a month; a sector asked
/// and answered is said, to be kept; one asked and not answered stays the one
/// known.
#[test]
fn test_the_canada_read_asks_a_sector_only_where_none_is_known_younger_than_a_month() {
    let changes = universes::parse_quotes(&quotes_answer());
    assert_eq!(changes.get("AAA"), Some(&Some(1.25)));
    assert_eq!(changes.get("CCC"), Some(&None), "no change stated");
    let now = 1_800_000_000.0;
    let month = universes::SECTOR_STALE_SEC;
    let mut known = std::collections::BTreeMap::new();
    known.insert("AAA".to_string(), ("Financials".to_string(), now - 60.0));
    known.insert("BBB".to_string(), ("Energy".to_string(), now - month - 1.0));
    let mut asked = Vec::new();
    let (rows, kept) = universes::canada_rows(&constituents(), &changes, &known, now, &mut |s: &str| {
        asked.push(s.to_string());
        if s == "CCC" { Some("Materials".to_string()) } else { None }
    });
    assert_eq!(asked, ["BBB", "CCC"], "AAA's sector is younger than a month; BBB's is older; CCC has none");
    assert_eq!(kept, [("CCC".to_string(), "Materials".to_string())]);
    let got: Vec<(&str, &str, Option<f64>)> = rows.iter().map(|r| (r.symbol.as_str(), r.sector.as_str(), r.percent_change)).collect();
    assert_eq!(got, [("AAA", "Financials", Some(1.25)), ("BBB", "Energy", Some(-0.5)), ("CCC", "Materials", None)], "BBB, asked and not answered, keeps the one known");
    // the next read, everything known and young: nothing asked
    let mut known_now = known.clone();
    for (s, sec) in [("BBB", "Energy"), ("CCC", "Materials")] {
        known_now.insert(s.to_string(), (sec.to_string(), now));
    }
    let mut asked_again = 0;
    universes::canada_rows(&constituents(), &changes, &known_now, now + 60.0, &mut |_| {
        asked_again += 1;
        None
    });
    assert_eq!(asked_again, 0);
    // a constituent TMX leaves unclassified: that answer is kept, and not asked
    // again on the next read
    let mut known_none = std::collections::BTreeMap::new();
    let (_, kept_none) = universes::canada_rows(&constituents()[2..], &changes, &known_none, now, &mut |_| Some("Not classified".to_string()));
    assert_eq!(kept_none, [("CCC".to_string(), "Not classified".to_string())]);
    for (s, sec) in kept_none {
        known_none.insert(s, (sec, now));
    }
    let mut asked_none = 0;
    universes::canada_rows(&constituents()[2..], &changes, &known_none, now + 1800.0, &mut |_| {
        asked_none += 1;
        None
    });
    assert_eq!(asked_none, 0, "Not classified, read within the month, is not asked again");
}
