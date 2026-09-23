//! Sector and country exposure, pinned end to end: each issuer's page parser,
//! a share's classification, the look-through (sources stood in for through
//! `exposure::hooks`, as `exposure.rs` exercises without network), and what
//! the store keeps and reads back through `feeds`, `rows` and `snapshot`. The
//! answers are held in `golden/exposure_records.json`, so a change of
//! representation must leave every figure as it was.
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-market --test exposure_records`,
//! and read the diff.

use bagholder_market::exposure::{self, hooks, Breakdown, Ctx, Holding, Listed, ShareClass, TmxSector};
use bagholder_market::htmltables::html_tables;
use bagholder_model::securities::Security;
use bagholder_store::feeds::{ExposureRecord, Weights};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, norm(v))).collect::<Map<_, _>>()),
        v => v,
    }
}

fn cell<T: serde::Serialize + ?Sized>(x: &T) -> Value {
    norm(serde_json::to_value(x).unwrap())
}

/// A record written through `exposure::store_exposure` (inside
/// `fund_exposure`/`refresh_security`) carries the real wall clock as its
/// `fetchedAt`, which a golden file cannot pin; every other field of it can
/// be.
fn drop_fetched_at(v: Value) -> Value {
    let mut m = v.as_object().cloned().unwrap_or_default();
    m.remove("fetchedAt");
    Value::Object(m)
}

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
    let pool = std::sync::Arc::new(bagholder_store::pool::Pool::new(&std::env::temp_dir().join("bh-exposure-golden-test-none.db")));
    Ctx { conn, pool, today: "2026-09-16".into() }
}

fn classified() -> HashMap<String, ShareClass> {
    HashMap::from([
        ("RY".to_string(), ShareClass { sector: "Financials".into(), industry: "Banking".into(), country: "Canada".into(), source: "TMX Money".into() }),
        ("SHOP".to_string(), ShareClass { sector: "Information Technology".into(), industry: "Software".into(), country: "Canada".into(), source: "TMX Money".into() }),
        ("PLTR".to_string(), ShareClass { sector: "Information Technology".into(), industry: "Software".into(), country: "United States".into(), source: "TMX Money".into() }),
    ])
}

fn set_classify(table: HashMap<String, ShareClass>) {
    hooks::CLASSIFY.with(|h| *h.borrow_mut() = Some(Box::new(move |sym, ex, _| {
        table.get(&sym.to_uppercase()).cloned()
            .unwrap_or_else(|| ShareClass { sector: String::new(), industry: String::new(), country: exposure::venue_country(ex), source: String::new() })
    })));
}

fn lookthrough(c: &Ctx, rows: &[Holding]) -> ExposureRecord {
    exposure::lookthrough(c, rows, 0, &mut Vec::new())
}

fn holding(ticker: &str, name: &str, weight: f64, sector: &str, country: &str, exchange: &str, currency: &str, fund: bool) -> Holding {
    Holding { ticker: ticker.into(), name: name.into(), weight, sector: sector.into(), country: country.into(), exchange: exchange.into(), currency: currency.into(), fund }
}

fn answers() -> Value {
    let mut out = Map::new();

    // --- the issuers' own page parsers ------------------------------------
    let (rows, as_of) = exposure::parse_ishares_csv(ISHARES_CSV);
    out.insert("parse_ishares_csv".into(), json!({"asOf": as_of, "rows": cell(&rows)}));

    let (sectors, holdings) = exposure::parse_evolve_page(EVOLVE_HTML);
    out.insert("parse_evolve_page".into(), json!({"sectors": cell(&sectors), "holdings": cell(&holdings)}));

    let (rows, rf) = exposure::parse_harvest_tables(&html_tables(HARVEST_TABLE_HTML));
    out.insert("parse_harvest_tables_plain".into(), json!({"referenceAsset": rf, "rows": cell(&rows)}));
    let (rows, rf) = exposure::parse_harvest_tables(&html_tables(HARVEST_NAMES_HTML));
    out.insert("parse_harvest_tables_single_stock".into(), json!({"referenceAsset": rf, "rows": cell(&rows)}));
    let (rows, rf) = exposure::parse_harvest_tables(&html_tables(HARVEST_FOF_HTML));
    out.insert("parse_harvest_tables_fund_of_funds".into(), json!({"referenceAsset": rf, "rows": cell(&rows)}));

    out.insert("parse_ninepoint_page".into(), cell(&exposure::parse_ninepoint_page(NINEPOINT_HTML)));

    let yahoo_data = json!({"quoteSummary": {"result": [{"topHoldings": {
        "holdings": [{"symbol": "AAPL", "holdingName": "Apple Inc", "holdingPercent": {"raw": 0.07}}, {"symbol": "RY.TO", "holdingName": "Royal Bank of Canada", "holdingPercent": {"raw": 0.03}}],
        "sectorWeightings": [{"realestate": {"raw": 0.02}}, {"technology": {"raw": 0.30}}, {"financial_services": {"raw": 0.20}}],
    }}]}});
    let (sectors, holdings) = exposure::parse_yahoo_summary(&yahoo_data);
    out.insert("parse_yahoo_summary".into(), json!({"sectors": cell(&sectors), "holdings": cell(&holdings)}));
    out.insert("parse_yahoo_summary_empty".into(), {
        let (sectors, holdings) = exposure::parse_yahoo_summary(&json!({}));
        json!({"sectors": cell(&sectors), "holdings": cell(&holdings)})
    });

    // --- names --------------------------------------------------------------
    out.insert("norm_sector_examples".into(), cell(&[
        exposure::norm_sector("Technology"), exposure::norm_sector("Financial"), exposure::norm_sector("Bitcoin Holding"),
        exposure::norm_sector("Cash and/or Derivatives"), exposure::norm_sector("Aerospace"),
    ]));
    out.insert("norm_country_examples".into(), cell(&[exposure::norm_country("USA"), exposure::norm_country("Korea, Republic of")]));
    out.insert("issuer_of_examples".into(), cell(&[
        exposure::issuer_of("Vanguard All-Equity ETF Portfolio - ETF"), exposure::issuer_of("iShares Core Equity ETF Portfolio"),
        exposure::issuer_of("Harvest Diversified High Income Shares ETF - Class A"), exposure::issuer_of("Ninepoint Partners LP - Cameco Highshares ETF"),
        exposure::issuer_of("Evolve All-in-One UltraYield ETF"), exposure::issuer_of("Shopify Inc."),
    ]));

    // --- classify_share, with the depositary-receipt retry ------------------
    {
        let conn = db();
        hooks::TMX_RECORD.with(|h| *h.borrow_mut() = Some(Box::new(|k| match k {
            "PLTR" => Some(TmxSector { symbol: "PLTR".into(), name: "Palantir CDR (CAD Hedged)".into(), sector: "Technology".into(), industry: "Software".into(), exchange_name: "Toronto Stock Exchange".into() }),
            "PLTR:US" => Some(TmxSector { symbol: "PLTR".into(), name: "Palantir Technologies Inc.".into(), sector: "Technology".into(), industry: "Software".into(), exchange_name: "Nasdaq Global Select".into() }),
            _ => None,
        })));
        out.insert("classify_share_cdr_retry".into(), cell(&exposure::classify_share(&ctx(&conn), "PLTR", "", "")));
    }
    {
        let conn = db();
        // no TMX record and not a US-shaped symbol: nothing is guessed
        out.insert("classify_share_nothing_found".into(), cell(&exposure::classify_share(&ctx(&conn), "ZZZ", "CSE", "CAD")));
    }

    // --- share_exposure, cached by ticker and venue --------------------------
    {
        let conn = db();
        set_classify(classified());
        let c = ctx(&conn);
        let rec = exposure::share_exposure(&c, "RY", "TSX", "CAD");
        out.insert("share_exposure_ry".into(), cell(&rec));
        // read back from the cache it just wrote, under the same key
        let cached = bagholder_store::feeds::exposure_record(&conn, "share:RY:").unwrap();
        out.insert("share_exposure_cache_key_hit".into(), json!(cached.is_some()));
    }

    // --- lookthrough: spread by weight, a fund held by a fund, a holding
    // named without a ticker, and a holding stated with its own sector -------
    {
        let conn = db();
        set_classify(classified());
        let rows = vec![
            holding("RY", "", 60.0, "", "", "TSX", "CAD", false),
            holding("ZZZ", "", 20.0, "", "", "", "", false),
            holding("SHOP", "", 20.0, "Information Technology", "Canada", "", "", false),
        ];
        out.insert("lookthrough_spread_by_weight".into(), cell(&lookthrough(&ctx(&conn), &rows)));
    }
    {
        let conn = db();
        hooks::CLASSIFY.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _| panic!("a stated holding is never classified"))));
        hooks::ADAPTER.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _, _| panic!("a stated holding is never looked through"))));
        let rows = vec![holding("IBIT", "iShares Bitcoin Trust ETF", 130.3, "Digital assets", "United States", "", "", true)];
        out.insert("lookthrough_stated_holding".into(), cell(&lookthrough(&ctx(&conn), &rows)));
    }
    {
        let conn = db();
        set_classify(classified());
        hooks::ADAPTER.with(|h| *h.borrow_mut() = Some(Box::new(|_, symbol, _, _| match symbol {
            "OUTER" => Some(Breakdown { sectors: Weights::default(), countries: Weights::default(), holdings: vec![
                holding("INNER", "Inner Index ETF", 50.0, "", "", "TSX", "CAD", true),
                holding("PLTR", "", 50.0, "", "", "NYSE", "USD", false)], source: "t".into(), as_of: String::new() }),
            "INNER" => Some(Breakdown { sectors: Weights::default(), countries: Weights::default(), holdings: vec![
                holding("RY", "", 100.0, "", "", "TSX", "CAD", false)], source: "t".into(), as_of: String::new() }),
            _ => None,
        })));
        let c = ctx(&conn);
        let rec = exposure::fund_exposure(&c, "OUTER", "Test Outer ETF", "TSX", 0, &mut Vec::new()).unwrap();
        out.insert("fund_exposure_fund_of_funds".into(), cell(&rec));
        let inner = bagholder_store::feeds::exposure_record(&conn, "fund:INNER").unwrap().unwrap();
        out.insert("fund_exposure_inner_fund_kept".into(), drop_fetched_at(cell(&inner)));
    }

    // --- fund_exposure: a family with no adapter falls back to Yahoo, and a
    // fund no source covers is stored as unclassified rather than dropped ----
    {
        let conn = db();
        set_classify(classified());
        hooks::FALLBACK.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _| Ok(Some(Breakdown {
            sectors: Weights(vec![("Financials".to_string(), 100.0)]), countries: Weights::default(),
            holdings: vec![holding("RY", "", 100.0, "", "", "TSX", "CAD", false)],
            source: "Yahoo Finance".into(), as_of: String::new(),
        })))));
        let rec = exposure::fund_exposure(&ctx(&conn), "ZZZ", "Someone Else Global Equity ETF", "TSX", 0, &mut Vec::new()).unwrap();
        out.insert("fund_exposure_yahoo_fallback".into(), cell(&rec));
    }
    {
        let conn = db();
        hooks::FALLBACK.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _| Err("down".into()))));
        let sec = Security { id: "sec-s-1".into(), symbol: "ZZZ".into(), name: "Nobody Fund ETF".into(), primary_exchange: "TSX".into(), currency: "CAD".into(), ..Default::default() };
        let rec = exposure::refresh_security(&ctx(&conn), &sec);
        out.insert("refresh_security_fund_no_source".into(), cell(&rec));
        out.insert("refresh_security_fund_no_source_stored".into(), drop_fetched_at(cell(&bagholder_store::feeds::exposure_record(&conn, "sec-s-1").unwrap().unwrap())));
    }

    // --- refresh_security for a plain share, and `stale` -----------------
    {
        let conn = db();
        set_classify(classified());
        let c = ctx(&conn);
        let sec = Security { id: "sec-s-ry".into(), symbol: "RY".into(), name: "Royal Bank of Canada".into(), primary_exchange: "TSX".into(), currency: "CAD".into(), ..Default::default() };
        let rec = exposure::refresh_security(&c, &sec);
        out.insert("refresh_security_share".into(), cell(&rec));
        out.insert("stale_ids".into(), cell(&exposure::stale(&c, &["sec-s-ry".to_string(), "sec-s-none".to_string()])));
    }

    // --- a fund named without a ticker: resolved, then looked through ------
    {
        let conn = db();
        hooks::ADAPTER.with(|h| *h.borrow_mut() = Some(Box::new(move |family, symbol, _, _| {
            assert_eq!(family, "harvest");
            if symbol == "APLE" {
                Some(Breakdown { sectors: Weights::default(), countries: Weights::default(), holdings: vec![holding("AAPL", "AAPL", 100.0, "", "", "", "USD", false)], source: "t".into(), as_of: String::new() })
            } else { None }
        })));
        hooks::RESOLVE.with(|h| *h.borrow_mut() = Some(Box::new(|_| Some(Listed { symbol: "APLE".into(), exchange: "TSX".into(), currency: "CAD".into() }))));
        let mut table = classified();
        table.insert("AAPL".into(), ShareClass { sector: "Information Technology".into(), industry: "Hardware".into(), country: "United States".into(), source: "TMX Money".into() });
        hooks::CLASSIFY.with(|h| *h.borrow_mut() = Some(Box::new(move |sym, _, _| {
            table.get(&sym.to_uppercase()).cloned().unwrap_or_else(|| ShareClass { sector: String::new(), industry: String::new(), country: String::new(), source: String::new() })
        })));
        let rows = vec![holding("", "Harvest Apple Enhanced High Income Shares ETF", 7.0, "", "", "", "", true)];
        out.insert("lookthrough_resolved_by_name".into(), cell(&lookthrough(&ctx(&conn), &rows)));
    }

    // --- the store round trip: replace_exposure / exposure_record / rows /
    // snapshot, all reading back the same rows -----------------------
    {
        let conn = db();
        let ry_rec = ExposureRecord {
            sectors: Weights(vec![("Financials".to_string(), 1.0)]), countries: Weights(vec![("Canada".to_string(), 1.0)]),
            coverage: 1.0, source: "TMX Money".into(), as_of: String::new(), industry: "Banking".into(), error: String::new(),
        };
        bagholder_store::feeds::replace_exposure(&conn, "share:RY:", &ry_rec, "2026-09-16T12:00:00Z").unwrap();
        let xeqt_rec = ExposureRecord {
            sectors: Weights(vec![("Financials".to_string(), 0.3), ("Information Technology".to_string(), 0.7)]),
            countries: Weights(vec![("Canada".to_string(), 0.6), ("United States".to_string(), 0.4)]),
            coverage: 0.9, source: "Yahoo Finance".into(), as_of: "2026-09-01".into(), industry: String::new(), error: String::new(),
        };
        bagholder_store::feeds::replace_exposure(&conn, "fund:XEQT", &xeqt_rec, "2026-09-16T12:00:00Z").unwrap();
        // a later write to the same key replaces it rather than merging
        bagholder_store::feeds::replace_exposure(&conn, "share:RY:", &ry_rec, "2026-09-17T09:00:00Z").unwrap();

        out.insert("store_exposure_record_ry".into(), cell(&bagholder_store::feeds::exposure_record(&conn, "share:RY:").unwrap().unwrap()));
        out.insert("store_exposure_record_missing".into(), cell(&bagholder_store::feeds::exposure_record(&conn, "nope").unwrap()));

        let snap_map = bagholder_store::feeds::all_exposures(&conn).unwrap();
        let mut snap_keys: Vec<&String> = snap_map.keys().collect();
        snap_keys.sort();
        out.insert("store_admin_exposures_map_keys".into(), cell(&snap_keys));
        out.insert("store_admin_exposures_map_xeqt".into(), cell(&snap_map["fund:XEQT"]));

        let rows_map = bagholder_store::rows::exposures(&conn).unwrap();
        let mut rows_out = Map::new();
        for (k, v) in rows_map.iter() {
            rows_out.insert(k.clone(), weights_cell(v));
        }
        out.insert("store_rows_exposures".into(), Value::Object(rows_out));

        let part = bagholder_store::feeds::all_exposures(&conn).unwrap();
        let mut part_keys: Vec<&String> = part.keys().collect();
        part_keys.sort();
        out.insert("store_snapshot_exposures_part_keys".into(), cell(&part_keys));
        out.insert("store_snapshot_exposures_part_ry".into(), cell(&part["share:RY:"]));
    }

    Value::Object(out)
}

/// The two weight maps of an exposure row as recorded by `bagholder_model`'s
/// `Exposure`, which has no `Serialize` of its own: {name: weight} pairs,
/// sorted by name so the golden file does not depend on map order.
fn weights_cell(e: &bagholder_model::exposure::Exposure) -> Value {
    let mut sectors: Vec<(String, f64)> = e.sectors.clone();
    let mut countries: Vec<(String, f64)> = e.countries.clone();
    sectors.sort_by(|a, b| a.0.cmp(&b.0));
    countries.sort_by(|a, b| a.0.cmp(&b.0));
    json!({"sectors": sectors, "countries": countries})
}

/// Pinned answers for `exposure.rs`'s page parsers, `classify_share`,
/// `share_exposure`, `lookthrough`, `fund_exposure`, `refresh_security`,
/// `stale`, and the store round trip through `feeds`, `rows` and
/// `snapshot`. To bless a change:
/// `BAGHOLDER_BLESS=1 cargo test -p bagholder-market --test exposure_records`.
#[test]
fn test_exposure_is_derived_and_stored_as_it_was() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/exposure_records.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/exposure_records.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}
