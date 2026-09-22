//! The issuers' parsers, the names folded onto one set,
//! the look-through (sources stood in for through `exposure::hooks`), and the
//! Portfolio's and Markets' slices.

const ISHARES_CSV: &str = r####"﻿Fund Holdings as of,"Sep 9, 2026"
 
Ticker,Name,Sector,Asset Class,Market Value,Weight (%),Notional Value,Shares,Price,Location,Exchange,Currency,FX Rate,Market Currency
"RY","ROYAL BANK OF CANADA","Financials","Equity","2,177,806,813.19","9.73","2,177,806,813.19","7,625,641.00","285.59","Canada","Toronto Stock Exchange","CAD","1.00","CAD"
"SHOP","SHOPIFY SUBORDINATE VOTING CLASS A","Information Technology","Equity","1,165,541,988.48","5.21","1,165,541,988.48","6,655,296.00","175.13","Canada","Toronto Stock Exchange","CAD","1.00","CAD"
"XEF","ISHARES MSCI EAFE IMI INDEX","Other","Equity","5,416,028,871.87","24.37","5,416,028,871.87","104,617,131.00","51.77","Canada","Toronto Stock Exchange","CAD","1.00","CAD"
"CAD","CAD CASH","Cash and/or Derivatives","Cash","31,678,794.26","0.14","31,678,794.26","31,678,794.00","100.00","Canada","-","CAD","1.00","CAD"
 
Fund Holdings as of,"Sep 9, 2026"
"####;

const EVOLVE_HTML: &str = r####"<html><body><script>
var portfolioBreakdownData = {"data":{"geographic":[{"name":"BIGY","weight":"46.33%"}],"sector":[{"name":"Technology","weight":"27.86%"},{"name":"Financial","weight":"24.63%"},{"name":"Communications","weight":"15.72%"}]}};
var holdingsData = {"data":[{"ticker":"MSFT US EQUITY","weight_percent":"4.11%","position":"6301","security_name":"Microsoft Corp","gics_sector":"Technology","country":"BIGY","last_price":"491.65","value":"4,278,213"},{"ticker":"RY CN EQUITY","weight_percent":"2.00%","security_name":"Royal Bank of Canada","gics_sector":"Financial","country":"Canada"}]};
</script></body></html>"####;

const HARVEST_TABLE_HTML: &str = r####"<table><tr><th>Name</th><th>Ticker</th><th>Weight</th><th>Sector</th><th>Country</th></tr>
<tr><td>iShares Bitcoin Trust ETF</td><td>IBIT US</td><td>130.3%</td><td>Bitcoin Holding</td><td>United States</td></tr>
<tr><td>Written Options</td><td></td><td>(4.5)%</td><td></td><td></td></tr>
<tr><td>Cash and other assets and liabilities</td><td></td><td>(25.8)%</td><td></td><td></td></tr></table>"####;

const HARVEST_NAMES_HTML: &str = r####"<table><tr><th>Fund Details</th><th>As at 2026/09/09</th></tr><tr><td>Ticker</td><td>PLTE</td></tr><tr><td>Reference Asset</td><td>PLTR</td></tr></table>
<table><tr><th>HOLDING</th><th>As at 2026/08/31</th></tr><tr><td>Palantir Technologies Inc.</td><td>128.2%</td></tr><tr><td>Written Options</td><td>(2.8)%</td></tr><tr><td>Cash and other assets and liabilities</td><td>(25.4)%</td></tr></table>"####;

const HARVEST_FOF_HTML: &str = r####"<table><tr><th>HOLDINGS</th><th>As at 2026/08/31</th></tr><tr><td>Harvest Apple Enhanced High Income Shares ETF</td><td>7.0%</td></tr><tr><td>Harvest NVIDIA Enhanced High Income Shares ETF</td><td>6.9%</td></tr></table>"####;

const NINEPOINT_HTML: &str = r####"<div><table><tr><td>Facts</td></tr><tr><td>Ticker</td><td>CCHI:TSX</td></tr><tr><td>Underlying Stock**</td><td>Cameco Corp. (CCO:TSX)</td></tr></table></div>"####;

use bagholder_market::exposure::{self, hooks, Ctx};
use bagholder_market::htmltables::html_tables;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::rc::Rc;

fn close(a: &Value, b: f64) {
    let a = a.as_f64().unwrap_or(f64::NAN);
    assert!((a - b).abs() < 1e-7, "{} != {}", a, b);
}

fn row(v: &Value, keys: &[&str]) -> Vec<Value> {
    keys.iter().map(|k| v[*k].clone()).collect()
}

// --- NamesTest

#[test]
fn test_sector_names_fold_onto_one_set() {
    assert_eq!(exposure::norm_sector("Technology"), "Information Technology");
    assert_eq!(exposure::norm_sector("Financial"), "Financials");
    assert_eq!(exposure::norm_sector("Communication"), "Communication Services");
    assert_eq!(exposure::norm_sector("Consumer, Non-cyclical"), "Consumer Staples");
    assert_eq!(exposure::norm_sector("Basic Materials"), "Materials");
    assert_eq!(exposure::norm_sector("Bitcoin Holding"), "Digital assets");
    assert_eq!(exposure::norm_sector("Cash and/or Derivatives"), "", "cash is no sector");
    assert_eq!(exposure::norm_sector("Aerospace"), "Aerospace", "a name outside the set passes through");
}

#[test]
fn test_country_names_and_venues() {
    assert_eq!(exposure::norm_country("USA"), "United States");
    assert_eq!(exposure::norm_country("Korea, Republic of"), "South Korea");
    assert_eq!(exposure::venue_country("TSX-V"), "Canada");
    assert_eq!(exposure::venue_country("NASDAQ"), "United States");
    assert_eq!(exposure::venue_country("OPRA"), "");
}

#[test]
fn test_issuer_of_a_fund_name() {
    assert_eq!(exposure::issuer_of("Vanguard All-Equity ETF Portfolio - ETF"), "vanguard");
    assert_eq!(exposure::issuer_of("iShares Core Equity ETF Portfolio"), "ishares");
    assert_eq!(exposure::issuer_of("Harvest Diversified High Income Shares ETF - Class A"), "harvest");
    assert_eq!(exposure::issuer_of("Ninepoint Partners LP - Cameco Highshares ETF"), "ninepoint");
    assert_eq!(exposure::issuer_of("Evolve All-in-One UltraYield ETF"), "evolve");
    assert_eq!(exposure::issuer_of("Shopify Inc."), "");
    assert!(exposure::is_fund("Global X High Interest Savings ETF"));
    assert!(!exposure::is_fund("Shopify Inc."));
}

// --- ParsersTest

#[test]
fn test_ishares_holdings_csv() {
    let (rows, as_of) = exposure::parse_ishares_csv(ISHARES_CSV);
    assert_eq!(as_of, "Sep 9, 2026");
    let got: Vec<Vec<Value>> = bagholder_model::testing::sent(&rows).iter().map(|r| row(r, &["ticker", "weight", "sector", "country", "fund"])).collect();
    assert_eq!(got, vec![
        vec![json!("RY"), json!(9.73), json!("Financials"), json!("Canada"), json!(false)],
        vec![json!("SHOP"), json!(5.21), json!("Information Technology"), json!("Canada"), json!(false)],
        vec![json!("XEF"), json!(24.37), json!(""), json!("Canada"), json!(true)],
    ], "cash is out; a fund row is marked to be looked through");
}

#[test]
fn test_evolve_page() {
    let (sectors, holdings) = exposure::parse_evolve_page(EVOLVE_HTML);
    assert_eq!(Value::Object(sectors), json!({"Information Technology": 27.86, "Financials": 24.63, "Communication Services": 15.72}));
    let got: Vec<Vec<Value>> = holdings.iter().map(|r| row(r, &["ticker", "weight", "sector", "country"])).collect();
    assert_eq!(got, vec![
        vec![json!("MSFT"), json!(4.11), json!("Information Technology"), json!("United States")],
        vec![json!("RY"), json!(2.0), json!("Financials"), json!("Canada")],
    ]);
}

#[test]
fn test_harvest_tables() {
    let (rows, rf) = exposure::parse_harvest_tables(&html_tables(HARVEST_TABLE_HTML));
    assert_eq!(rf, "");
    let got: Vec<Vec<Value>> = bagholder_model::testing::sent(&rows).iter().map(|r| row(r, &["ticker", "weight", "sector", "country"])).collect();
    assert_eq!(got, vec![vec![json!("IBIT"), json!(130.3), json!("Digital assets"), json!("United States")]], "options and cash rows are out");
    let (rows, rf) = exposure::parse_harvest_tables(&html_tables(HARVEST_NAMES_HTML));
    assert_eq!(rf, "PLTR");
    let got: Vec<Vec<Value>> = bagholder_model::testing::sent(&rows).iter().map(|r| row(r, &["name", "weight", "fund"])).collect();
    assert_eq!(got, vec![vec![json!("Palantir Technologies Inc."), json!(128.2), json!(false)]]);
    let (rows, _) = exposure::parse_harvest_tables(&html_tables(HARVEST_FOF_HTML));
    let got: Vec<Vec<Value>> = bagholder_model::testing::sent(&rows).iter().map(|r| row(r, &["name", "weight", "fund"])).collect();
    assert_eq!(got, vec![
        vec![json!("Harvest Apple Enhanced High Income Shares ETF"), json!(7.0), json!(true)],
        vec![json!("Harvest NVIDIA Enhanced High Income Shares ETF"), json!(6.9), json!(true)],
    ]);
}

#[test]
fn test_ninepoint_page() {
    assert_eq!(exposure::parse_ninepoint_page(NINEPOINT_HTML), ("CCHI".to_string(), "CCO".to_string(), "TSX".to_string()));
}

#[test]
fn test_yahoo_summary() {
    let data = json!({"quoteSummary": {"result": [{"topHoldings": {
        "holdings": [{"symbol": "AAPL", "holdingName": "Apple Inc", "holdingPercent": {"raw": 0.07}}, {"symbol": "RY.TO", "holdingName": "Royal Bank of Canada", "holdingPercent": {"raw": 0.03}}],
        "sectorWeightings": [{"realestate": {"raw": 0.02}}, {"technology": {"raw": 0.30}}, {"financial_services": {"raw": 0.20}}],
    }}]}});
    let (sectors, holdings) = exposure::parse_yahoo_summary(&data);
    assert_eq!(Value::Object(sectors), json!({"Real Estate": 2.0, "Information Technology": 30.0, "Financials": 20.0}));
    let got: Vec<Vec<Value>> = holdings.iter().map(|r| row(r, &["ticker", "weight", "exchange"])).collect();
    assert_eq!(got, vec![vec![json!("AAPL"), json!(7.0), json!("")], vec![json!("RY.TO"), json!(3.0), json!("TSX")]]);
}

// --- LookthroughTest

fn db() -> Connection {
    hooks::clear();
    // no test here may reach a real source
    hooks::FALLBACK.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _| Ok(None))));
    hooks::TMX_RECORD.with(|h| *h.borrow_mut() = Some(Box::new(|_| None)));
    let conn = Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&conn).unwrap();
    conn
}

fn ctx(conn: &Connection) -> Ctx<'_> {
    let pool = std::sync::Arc::new(bagholder_store::pool::Pool::new(&std::env::temp_dir().join("bh-exposure-test-none.db")));
    Ctx { conn, pool, today: "2026-09-16".into() }
}

fn classified() -> Map<String, Value> {
    json!({"RY": {"sector": "Financials", "industry": "Banking", "country": "Canada", "source": "TMX Money"},
           "SHOP": {"sector": "Information Technology", "industry": "Software", "country": "Canada", "source": "TMX Money"},
           "PLTR": {"sector": "Information Technology", "industry": "Software", "country": "United States", "source": "TMX Money"}})
        .as_object().cloned().unwrap()
}

fn set_classify(table: Map<String, Value>) {
    hooks::CLASSIFY.with(|h| *h.borrow_mut() = Some(Box::new(move |sym, ex, _| {
        table.get(&sym.to_uppercase()).cloned()
            .unwrap_or_else(|| json!({"sector": "", "industry": "", "country": exposure::venue_country(ex), "source": ""}))
    })));
}

fn lookthrough(c: &Ctx, rows: &[Value]) -> Value {
    exposure::lookthrough(c, rows, 0, &mut Vec::new())
}

#[test]
fn test_holdings_are_spread_by_weight_and_the_rest_is_unclassified() {
    let conn = db();
    set_classify(classified());
    let rows = vec![
        json!({"ticker": "RY", "name": "", "weight": 60, "sector": "", "country": "", "exchange": "TSX", "currency": "CAD", "fund": false}),
        json!({"ticker": "ZZZ", "name": "", "weight": 20, "sector": "", "country": "", "exchange": "", "currency": "", "fund": false}),
        json!({"ticker": "SHOP", "name": "", "weight": 20, "sector": "Information Technology", "country": "Canada", "exchange": "", "currency": "", "fund": false}),
    ];
    let agg = lookthrough(&ctx(&conn), &rows);
    close(&agg["sectors"]["Financials"], 0.6);
    close(&agg["sectors"]["Information Technology"], 0.2);
    close(&agg["countries"]["Canada"], 0.8);
    close(&agg["coverage"], 0.8);
}

#[test]
fn test_a_fund_held_by_a_fund_is_looked_through() {
    let conn = db();
    set_classify(classified());
    hooks::ADAPTER.with(|h| *h.borrow_mut() = Some(Box::new(|_, symbol, _, _| match symbol {
        "OUTER" => Some(json!({"sectors": {}, "countries": {}, "holdings": [
            {"ticker": "INNER", "name": "Inner Index ETF", "weight": 50, "sector": "", "country": "", "exchange": "TSX", "currency": "CAD", "fund": true},
            {"ticker": "PLTR", "name": "", "weight": 50, "sector": "", "country": "", "exchange": "NYSE", "currency": "USD", "fund": false}], "source": "t", "asOf": ""})),
        "INNER" => Some(json!({"sectors": {}, "countries": {}, "holdings": [
            {"ticker": "RY", "name": "", "weight": 100, "sector": "", "country": "", "exchange": "TSX", "currency": "CAD", "fund": false}], "source": "t", "asOf": ""})),
        _ => None,
    })));
    let c = ctx(&conn);
    let rec = exposure::fund_exposure(&c, "OUTER", "Test Outer ETF", "TSX", 0, &mut Vec::new()).unwrap();
    close(&rec["sectors"]["Financials"], 0.5);
    close(&rec["sectors"]["Information Technology"], 0.5);
    close(&rec["countries"]["Canada"], 0.5);
    close(&rec["countries"]["United States"], 0.5);
    assert_eq!(rec["coverage"], json!(1.0));
    let inner = bagholder_store::feeds::exposure_record(&conn, "fund:INNER").unwrap().unwrap();
    assert_eq!(inner["coverage"], json!(1.0), "the inner fund's record is kept for the next fund that holds it");
}

#[test]
fn test_a_holding_stated_with_sector_and_country_is_taken_as_stated() {
    let conn = db();
    hooks::CLASSIFY.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _| panic!("must not classify"))));
    hooks::ADAPTER.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _, _| panic!("must not look it through"))));
    let rows = vec![json!({"ticker": "IBIT", "name": "iShares Bitcoin Trust ETF", "weight": 130.3, "sector": "Digital assets", "country": "United States", "exchange": "", "currency": "", "fund": true})];
    let agg = lookthrough(&ctx(&conn), &rows);
    assert_eq!((agg["sectors"].clone(), agg["countries"].clone(), agg["coverage"].clone()),
               (json!({"Digital assets": 1.0}), json!({"United States": 1.0}), json!(1.0)));
}

#[test]
fn test_a_fund_named_without_a_ticker_is_resolved_then_looked_through() {
    let conn = db();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let s2 = seen.clone();
    hooks::ADAPTER.with(|h| *h.borrow_mut() = Some(Box::new(move |family, symbol, _, _| {
        assert_eq!(family, "harvest");
        s2.borrow_mut().push(symbol.to_string());
        if symbol == "APLE" {
            Some(json!({"sectors": {}, "countries": {}, "holdings": [{"ticker": "AAPL", "name": "AAPL", "weight": 100.0, "sector": "", "country": "", "exchange": "", "currency": "USD", "fund": false}], "source": "t", "asOf": ""}))
        } else { None }
    })));
    hooks::RESOLVE.with(|h| *h.borrow_mut() = Some(Box::new(|_| Some(json!({"symbol": "APLE", "exchange": "TSX", "currency": "CAD"})))));
    let mut table = classified();
    table.insert("AAPL".into(), json!({"sector": "Information Technology", "industry": "Hardware", "country": "United States", "source": "TMX Money"}));
    hooks::CLASSIFY.with(|h| *h.borrow_mut() = Some(Box::new(move |sym, _, _| {
        table.get(&sym.to_uppercase()).cloned().unwrap_or_else(|| json!({"sector": "", "industry": "", "country": "", "source": ""}))
    })));
    let rows = vec![json!({"ticker": "", "name": "Harvest Apple Enhanced High Income Shares ETF", "weight": 7.0, "sector": "", "country": "", "exchange": "", "currency": "", "fund": true})];
    let agg = lookthrough(&ctx(&conn), &rows);
    assert_eq!(*seen.borrow(), vec!["APLE".to_string()], "the fund is looked through under the ticker the directory gave");
    assert_eq!((agg["sectors"].clone(), agg["countries"].clone()), (json!({"Information Technology": 1.0}), json!({"United States": 1.0})));
}

#[test]
fn test_a_bare_ticker_answered_with_a_depositary_receipt_is_retried_as_the_us_listing() {
    let conn = db();
    hooks::TMX_RECORD.with(|h| *h.borrow_mut() = Some(Box::new(|k| match k {
        "PLTR" => Some(json!({"name": "Palantir CDR (CAD Hedged)", "sector": "Technology", "industry": "Software", "exchangeName": "Toronto Stock Exchange"})),
        "PLTR:US" => Some(json!({"name": "Palantir Technologies Inc.", "sector": "Technology", "industry": "Software", "exchangeName": "Nasdaq Global Select"})),
        _ => None,
    })));
    let c = exposure::classify_share(&ctx(&conn), "PLTR", "", "");
    assert_eq!((c["sector"].clone(), c["country"].clone()), (json!("Information Technology"), json!("United States")));
}

#[test]
fn test_a_family_without_an_adapter_falls_back_to_yahoo() {
    let conn = db();
    set_classify(classified());
    hooks::FALLBACK.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _| Ok(Some(json!({"sectors": {"Financials": 100.0}, "countries": {},
        "holdings": [{"ticker": "RY", "name": "", "weight": 100, "sector": "", "country": "", "exchange": "TSX", "currency": "CAD", "fund": false}],
        "source": "Yahoo Finance", "asOf": ""}))))));
    let rec = exposure::fund_exposure(&ctx(&conn), "ZZZ", "Someone Else Global Equity ETF", "TSX", 0, &mut Vec::new()).unwrap();
    assert_eq!(rec["sectors"], json!({"Financials": 1.0}), "the fund's stated sectors");
    assert_eq!(rec["countries"], json!({"Canada": 1.0}), "the countries from its named holdings");
    assert_eq!(rec["source"], "Yahoo Finance");
}

#[test]
fn test_a_fund_no_source_covers_is_stored_as_unclassified() {
    let conn = db();
    hooks::FALLBACK.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _| Err("down".into()))));
    let rec = exposure::refresh_security(&ctx(&conn), &json!({"id": "sec-s-1", "symbol": "ZZZ", "name": "Nobody Fund ETF", "primaryExchange": "TSX", "currency": "CAD"}));
    assert_eq!((rec["sectors"].clone(), rec["countries"].clone(), rec["coverage"].clone()), (json!({}), json!({}), json!(0.0)));
    assert_eq!(bagholder_store::feeds::exposure_record(&conn, "sec-s-1").unwrap().unwrap()["coverage"], json!(0.0));
}

#[test]
fn test_a_share_is_its_one_sector_and_country() {
    let conn = db();
    set_classify(classified());
    let c = ctx(&conn);
    let rec = exposure::refresh_security(&c, &json!({"id": "sec-s-ry", "symbol": "RY", "name": "Royal Bank of Canada", "primaryExchange": "TSX", "currency": "CAD"}));
    assert_eq!((rec["sectors"].clone(), rec["countries"].clone(), rec["coverage"].clone()), (json!({"Financials": 1.0}), json!({"Canada": 1.0}), json!(1.0)));
    assert_eq!(exposure::stale(&c, &["sec-s-ry".to_string(), "sec-s-none".to_string()]), vec!["sec-s-none".to_string()]);
}

// --- PortfolioSlicesTest

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

fn named(v: &[bagholder_model::wire::ExposureSlice]) -> Vec<(String, f64)> {
    v.iter().map(|s| (s.name.clone(), (s.value * 100.0).round() / 100.0)).collect()
}

/// Holdings sketched from a few fields.
fn held(rows: &[Value]) -> Vec<bagholder_model::wire::Position> {
    rows.iter().cloned().map(bagholder_model::testing::holding).collect()
}

/// Exposure records as the model reads them.
fn read(exposures: &Map<String, Value>) -> bagholder_model::exposure::Exposures {
    exposures.iter().map(|(k, v)| (k.clone(), serde_json::from_value(v.clone()).unwrap())).collect()
}

#[test]
fn test_positions_spread_by_their_records() {
    let positions = vec![
        json!({"mv": 1000.0, "currency": "CAD", "securityId": "a", "short": false, "kind": "Shares"}),
        json!({"mv": 500.0, "currency": "USD", "securityId": "b", "short": false, "kind": "Shares"}),
        json!({"mv": 300.0, "currency": "CAD", "securityId": "c", "short": false, "kind": "Shares"}),
        json!({"mv": 100.0, "currency": "CAD", "securityId": "a", "short": true, "kind": "Shares"}),
        json!({"mv": 200.0, "currency": "CAD", "securityId": "btc", "short": false, "kind": "Crypto"}),
        json!({"mv": 50.0, "currency": "USD", "securityId": "opt", "short": true, "kind": "Options", "underlying": "AAPL"}),
    ];
    let exposures = obj(json!({"a": {"sectors": {"Financials": 1.0}, "countries": {"Canada": 1.0}, "coverage": 1.0},
        "b": {"sectors": {"Information Technology": 0.5, "Energy": 0.25}, "countries": {"United States": 0.75}, "coverage": 0.75},
        "share:AAPL::US": {"sectors": {"Information Technology": 1.0}, "countries": {"United States": 1.0}, "coverage": 1.0}}));
    let cad = |v: f64, c: &str| v * if c == "USD" { 2.0 } else { 1.0 };
    let (sectors, regions) = bagholder_model::exposure::exposure_slices(&held(&positions).iter().collect::<Vec<_>>(), &read(&exposures), &cad);
    let f = |n: &str, v: f64| (n.to_string(), v);
    assert_eq!(named(&sectors), vec![f("Financials", 1100.0), f("Information Technology", 600.0), f("Energy", 250.0), f("Digital assets", 200.0), f("Not classified", 550.0)]);
    let total: f64 = sectors.iter().map(|s| s.share).sum();
    assert!((total - 1.0).abs() < 1e-7);
    assert_eq!(named(&regions), vec![f("Canada", 1100.0), f("United States", 850.0), f("Not classified", 750.0)], "a coin has no country");
}

#[test]
fn test_a_stored_alias_folds_when_read() {
    let positions = vec![json!({"mv": 100.0, "currency": "CAD", "securityId": "a", "short": false, "kind": "Shares"}),
                         json!({"mv": 100.0, "currency": "CAD", "securityId": "b", "short": false, "kind": "Shares"})];
    let exposures = obj(json!({"a": {"sectors": {"Communication": 1.0}, "countries": {}, "coverage": 1.0},
                               "b": {"sectors": {"Communication Services": 1.0}, "countries": {}, "coverage": 1.0}}));
    let (sectors, _) = bagholder_model::exposure::exposure_slices(&held(&positions).iter().collect::<Vec<_>>(), &read(&exposures), &|v, _| v);
    assert_eq!(named(&sectors), vec![("Communication Services".to_string(), 200.0)]);
}

// --- MarketsTest

#[test]
fn test_heatmap_tiles_take_the_dominant_sector() {
    let positions = vec![
        json!({"id": "p1", "mv": 1000.0, "currency": "CAD", "securityId": "a", "short": false, "kind": "Shares", "symbol": "XEQT", "exchange": "TSX", "percentChange": 0.4}),
        json!({"id": "p2", "mv": 500.0, "currency": "USD", "securityId": "b", "short": false, "kind": "Shares", "symbol": "NVDA", "exchange": "NASDAQ", "percentChange": -1.2}),
        json!({"id": "p3", "mv": 200.0, "currency": "CAD", "securityId": "btc", "short": false, "kind": "Crypto", "symbol": "BTC", "exchange": "Crypto", "percentChange": null}),
        json!({"id": "p4", "mv": 50.0, "currency": "USD", "securityId": "opt", "short": false, "kind": "Options", "symbol": "AAPL 20DEC26 200.00 CALL", "underlying": "AAPL", "exchange": "", "percentChange": 3.0}),
        json!({"id": "p5", "mv": 0.0, "currency": "CAD", "securityId": "z", "short": false, "kind": "Shares", "symbol": "ZERO", "exchange": "TSX", "percentChange": null}),
        json!({"id": "p6", "mv": 250.0, "currency": "USD", "securityId": "b", "short": false, "kind": "Shares", "symbol": "NVDA", "exchange": "NASDAQ", "percentChange": -1.2}),
    ];
    let exposures = obj(json!({"a": {"sectors": {"Financials": 0.3, "Information Technology": 0.45, "Energy": 0.25}},
                               "b": {"sectors": {"Information Technology": 1.0}},
                               "share:AAPL::US": {"sectors": {"Information Technology": 1.0}}}));
    let tiles = bagholder_model::markets::heatmap_items(&held(&positions).iter().collect::<Vec<_>>(), &read(&exposures), &|v, c| v * if c == "USD" { 2.0 } else { 1.0 });
    let got: Vec<Vec<Value>> = bagholder_model::testing::sent(&tiles).iter().map(|t| row(t, &["symbol", "value", "sector", "percentChange"])).collect();
    assert_eq!(got, vec![
        vec![json!("XEQT"), json!(1000.0), json!("Information Technology"), json!(0.4)],
        vec![json!("NVDA"), json!(1500.0), json!("Information Technology"), json!(-1.2)],
        vec![json!("BTC"), json!(200.0), json!("Digital assets"), Value::Null],
        vec![json!("AAPL 20DEC26 200.00 CALL"), json!(100.0), json!("Information Technology"), json!(3.0)],
    ]);
}

#[test]
fn test_watch_rows_carry_the_quote_the_sector_and_the_holding() {
    let snapshot = json!({"exposures": {"share:SHOP:": {"sectors": {"Information Technology": 1.0}}},
        "watchlist": [{"symbol": "SHOP", "exchange": "TSX", "name": "Shopify Inc.", "currency": "CAD"}, {"symbol": "RKLB", "exchange": "NASDAQ", "name": "Rocket Lab", "currency": "USD"}]});
    let market = json!({"quotes": {"SHOP@TSX": {"price": 212.06, "priceChange": 1.56, "percentChange": 0.74}}});
    let base = bagholder_model::base::build_base(&snapshot, &market, &Default::default(), Some("2026-09-16"));
    let positions = vec![json!({"id": "p9", "symbol": "SHOP", "exchange": "TSX"})];
    let rows = bagholder_model::markets::watch_rows(&base, &held(&positions).iter().collect::<Vec<_>>());
    let got: Vec<Vec<Value>> = bagholder_model::testing::sent(&rows).iter().map(|r| row(r, &["symbol", "last", "percentChange", "sector", "positionId"])).collect();
    assert_eq!(got, vec![
        vec![json!("SHOP"), json!(212.06), json!(0.74), json!("Information Technology"), json!("p9")],
        vec![json!("RKLB"), Value::Null, Value::Null, json!("Not classified"), Value::Null],
    ]);
}
