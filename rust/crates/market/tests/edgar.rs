//! EDGAR. No network: the ticker map is primed and the
//! submissions, index and document requests are given as closures.
use bagholder_core::ListingKey;
use bagholder_market::disclosures::{self as d, Fetched, Gathered, Regulators, SourceError};
use bagholder_market::edgar;
use bagholder_store::feeds::{FiledDocument, Filer, Regulator};
use serde_json::{json, Value};
use std::cell::RefCell;

fn submissions() -> Value {
    json!({
        "name": "NVIDIA CORP",
        "filings": {"recent": {
            "form": ["10-Q", "8-K", "4", "SCHEDULE 13D/A", "424B5", "DEF 14A", "NT 10-K"],
            "filingDate": ["2026-08-05", "2026-08-01", "2026-07-30", "2026-07-20", "2026-07-10", "2026-06-15", "2026-06-01"],
            "primaryDocument": ["nvda-10q.htm", "nvda-8k.htm", "form4.xml", "sc13da.htm", "424b5.htm", "proxy.htm", ""],
            "accessionNumber": ["0001-26-01", "0001-26-02", "0001-26-03", "0001-26-04", "0001-26-05", "0001-26-06", "0001-26-07"],
            "primaryDocDescription": ["", "", "", "", "", "", ""],
        }},
    })
}

/// The SEC's ticker list as it publishes it: fields, then rows.
fn ticker_list() -> Value {
    json!({
        "fields": ["cik", "name", "ticker", "exchange"],
        "data": [
            [1045810, "NVIDIA CORP", "NVDA", "Nasdaq"],
            [1594805, "SHOPIFY INC.", "SHOP", "NYSE"],
            [1067983, "BERKSHIRE HATHAWAY INC", "BRK-B", "NYSE"],
            [9999, "SOME OTC CO", "OTCX", "OTC"],
        ],
    })
}

/// The SEC's list is one per process: the tests that set it take turns.
fn the_list() -> std::sync::MutexGuard<'static, ()> {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    TURN.lock().unwrap_or_else(|e| e.into_inner())
}

fn set_up() -> std::sync::MutexGuard<'static, ()> {
    let turn = the_list();
    edgar::set_ticker_map(edgar::parse_tickers(&ticker_list()).unwrap());
    turn
}

fn subs(_: &str) -> Fetched<Value> {
    Ok(submissions())
}

fn fetch(cik: i64) -> Vec<FiledDocument> {
    edgar::fetch_with(cik, 200, &subs).unwrap()
}

/// A minimal document, with everything but `form` and `url` blank.
fn doc(source: Regulator, form: &str, url: &str) -> FiledDocument {
    FiledDocument {
        id: String::new(), source, category: String::new(), profile_no: String::new(), issuer: String::new(),
        form: form.into(), title: String::new(), date: String::new(), date_text: String::new(), size: String::new(),
        url: url.into(),
    }
}

fn on(symbol: &str, venue: &str) -> ListingKey {
    ListingKey::on(symbol, venue).unwrap()
}

// --- EdgarUnitTest

#[test]
fn test_a_listing_is_written_as_the_sec_writes_its_ticker() {
    assert_eq!(edgar::sec_ticker(&on("BRK.B", "NYSE")), "BRK-B");
    assert_eq!(edgar::sec_ticker(&on("nvda", "NASDAQ")), "NVDA");
}

#[test]
fn test_forms_map_to_the_shared_categories() {
    assert_eq!(edgar::category("10-Q"), d::FINANCIALS);
    assert_eq!(edgar::category("40-F"), d::FINANCIALS);
    assert_eq!(edgar::category("8-K"), d::EVENTS);
    assert_eq!(edgar::category("DEF 14A"), d::GOVERNANCE);
    assert_eq!(edgar::category("424B5"), d::OFFERINGS);
    assert_eq!(edgar::category("4"), d::INSIDER);
    assert_eq!(edgar::category("SCHEDULE 13D/A"), d::INSIDER);
    assert_eq!(edgar::category("NT 10-K"), d::OTHER);
}

/// A US listing's filer is the SEC's own for its ticker, on an exchange or over
/// the counter, whatever exchange the row names; a listing on another market is
/// never matched to a US filer by its ticker.
#[test]
fn test_a_us_listing_is_its_secs_filer_and_no_other_market_is_matched_by_ticker() {
    let _turn = set_up();
    let cik = |k: ListingKey| edgar::filer_of(&k).unwrap().map(|f| f.0);
    assert_eq!(cik(on("NVDA", "NASDAQ")), Ok(1045810));
    assert_eq!(cik(on("BRK.B", "NYSE")), Ok(1067983));
    assert!(cik(on("SHOP", "TSX")).is_err(), "the TSX listing reaches the SEC only through the interlisted list");
    assert!(cik(on("ZZZZ", "NASDAQ")).is_err());
    assert_eq!(cik(on("OTCX", "OTC")), Ok(9999), "an OTC listing is the SEC's OTC row");
    assert_eq!(cik(on("OTCX", "NYSE")), Ok(9999), "one ticker is one issue across the US market, whatever the row names");
}

/// The SEC's list as published on 2026-10-16, recorded: every row is read, those
/// naming a newer exchange (Texas) or none among them, and each ticker it holds
/// is its one filer.
#[test]
fn every_row_of_the_secs_recorded_list_is_its_tickers_filer() {
    use std::io::Read;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/sec/company_tickers_exchange-2026-10-16.json.gz");
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap()).read_to_string(&mut text).unwrap();
    let data: Value = serde_json::from_str(&text).unwrap();
    let rows = data["data"].as_array().unwrap();
    let t = edgar::parse_tickers(&data).unwrap();
    for row in rows {
        let ticker = row[2].as_str().unwrap().trim().to_uppercase();
        assert_eq!(t[&ticker].iter().map(|f| f.0).collect::<Vec<_>>(), [row[0].as_i64().unwrap()], "{row}");
    }
    assert_eq!(t.len(), rows.len(), "each ticker once");
    let _turn = the_list();
    // the rows a filter by exchange dropped: the Texas exchange's, and those naming none
    for (ticker, cik) in [("ET", 1276187), ("DDS", 28917), ("SUN", 1552275), ("PHD", 1305767)] {
        assert_eq!(t[ticker].iter().map(|f| f.0).collect::<Vec<_>>(), [cik], "{ticker}");
    }
    edgar::set_ticker_map(t);
    assert_eq!(edgar::filer_of(&on("ET", "TXSE")).unwrap().map(|f| f.0), Ok(1276187));
    assert_eq!(edgar::filer_of(&on("PHD", "NYSE")).unwrap().map(|f| f.0), Ok(1305767));
}

/// The list is read again once it is as old as a listing's filers stand, so the
/// daily re-check of a filer reads that day's list; a read that fails is the failure.
#[test]
fn the_secs_list_is_read_again_each_day() {
    let _turn = the_list();
    let list = |cik: i64| json!({"fields": ["cik", "name", "ticker", "exchange"], "data": [[cik, "X CO", "X", "NYSE"]]});
    let day = std::time::Duration::from_secs_f64(d::FILERS_RECHECK_HOURS * 3600.0);
    // past any list another test set
    let start = std::time::SystemTime::now() + day * 2;
    let x = |t: &edgar::Tickers| t["X"][0].0;
    assert_eq!(x(&edgar::ticker_map_at(start, || Ok(list(1))).unwrap()), 1);
    assert_eq!(x(&edgar::ticker_map_at(start + day / 2, || panic!("read again within the day")).unwrap()), 1);
    assert_eq!(x(&edgar::ticker_map_at(start + day, || Ok(list(2))).unwrap()), 2, "a day on: today's list");
    assert!(edgar::ticker_map_at(start + day * 2, || Err(SourceError::Unavailable("down".into()))).is_err(), "a failed read is said, not yesterday's list");
    // the failure rests for every listing that asks: said again, not read again, until
    // its rest is over, and the rest grows with each failure in a row
    let failed = start + day * 2;
    let rest = d::FAILED_READ_REST;
    let again = edgar::ticker_map_at(failed + rest / 2, || panic!("read again within its rest"));
    assert!(again.unwrap_err().to_string().contains("down"));
    assert!(edgar::ticker_map_at(failed + rest, || Err(SourceError::Unavailable("still down".into()))).is_err());
    let second = failed + rest;
    assert!(edgar::ticker_map_at(second + rest, || panic!("twice the rest after a second failure")).is_err());
    assert_eq!(x(&edgar::ticker_map_at(second + rest * 2, || Ok(list(3))).unwrap()), 3, "past the grown rest: read, and read");
    // a read refused offline is no failure: nothing rests
    let later = second + rest * 2 + day;
    assert!(matches!(edgar::ticker_map_at(later, || Err(SourceError::Offline)), Err(SourceError::Offline)));
    assert_eq!(x(&edgar::ticker_map_at(later, || Ok(list(4))).unwrap()), 4, "asked again at once");
}

/// A ticker the list names two filers for is neither's.
#[test]
fn a_ticker_the_list_names_twice_is_no_ones() {
    let _turn = the_list();
    let t = edgar::parse_tickers(&json!({"fields": ["cik", "name", "ticker", "exchange"], "data": [[1, "A", "DUP", "NYSE"], [2, "B", "DUP", "OTC"], [3, "C", "ONE", null], [3, "C", "ONE", "NYSE"]]})).unwrap();
    assert_eq!(t["DUP"].len(), 2);
    assert_eq!(t["ONE"].len(), 1, "the same filer twice is one");
    edgar::set_ticker_map(t);
    assert!(edgar::filer_of(&on("DUP", "NYSE")).unwrap().unwrap_err().contains("more than one"));
}

#[test]
fn test_a_ticker_list_that_is_not_the_secs_shape_does_not_read() {
    for bad in [json!({}), json!({"fields": ["cik", "ticker"], "data": []}), json!([])] {
        assert!(edgar::parse_tickers(&bad).is_err(), "{bad}");
    }
}

#[test]
fn test_fetch_normalizes_rows_with_source_and_url() {
    let items = fetch(1045810);
    assert_eq!(items.len(), 7);
    let first = &items[0];
    assert_eq!(first.source, Regulator::Sec);
    assert_eq!(first.id, "sec:0001-26-01");
    assert_eq!(first.form, "10-Q");
    assert_eq!(first.category, d::FINANCIALS);
    assert!(first.url.starts_with("https://www.sec.gov/Archives/edgar/data/1045810/000126"));
}

#[test]
fn test_a_missing_document_url_falls_back_to_the_company_page() {
    let items = fetch(1045810);
    let nt = items.iter().find(|i| i.form == "NT 10-K").unwrap();
    assert!(nt.url.contains("browse-edgar"));
}

// --- the gathering of a listing's regulators

/// Two regulators standing in, each with its documents or its failure.
struct Regs {
    sedar: Fetched<Vec<FiledDocument>>,
    sec: Fetched<Vec<FiledDocument>>,
}

impl Regulators for Regs {
    fn filer(&self, listing: &ListingKey, _: &str) -> Option<(Regulator, Fetched<Result<String, String>>)> {
        if listing.us() {
            Some((Regulator::Sec, Ok(Ok("1".into()))))
        } else if listing.canadian() {
            Some((Regulator::Sedar, Ok(Ok("000000001".into()))))
        } else {
            None
        }
    }
    fn documents(&self, filer: &Filer, _: usize) -> Fetched<Vec<FiledDocument>> {
        match filer.source {
            Regulator::Sedar => self.sedar.clone(),
            Regulator::Sec => self.sec.clone(),
        }
    }
    fn interlisted(&self) -> Fetched<(String, Vec<bagholder_store::feeds::Interlisted>)> {
        Ok(("2026-10-01".into(), vec![]))
    }
}

fn ids(out: &Gathered) -> Vec<String> {
    out.items.iter().map(|i| i.id.clone()).collect()
}

fn dated(source: Regulator, id: &str, date: &str) -> FiledDocument {
    FiledDocument { id: id.into(), date: date.into(), ..doc(source, "", "") }
}

#[test]
fn test_a_listing_and_its_interlisted_partner_merge_newest_first() {
    let r = Regs { sedar: Ok(vec![dated(Regulator::Sedar, "a:1", "2026-01-01")]), sec: Ok(vec![dated(Regulator::Sec, "b:1", "2026-05-01")]) };
    let out = d::gather(&r, &on("X", "TSX"), Some(&on("X", "NYSE")), "2026-09-16", 200);
    assert_eq!(ids(&out), ["b:1", "a:1"]);
    assert!(out.sources[&Regulator::Sedar].matched && out.sources[&Regulator::Sec].matched);
    // unpaired, the listing shows its own regulator's alone
    let alone = d::gather(&r, &on("X", "TSX"), None, "2026-09-16", 200);
    assert_eq!(ids(&alone), ["a:1"]);
    assert!(!alone.sources.contains_key(&Regulator::Sec));
}

#[test]
fn test_a_failing_regulator_is_recorded_and_the_other_still_returns() {
    let r = Regs { sedar: Err(SourceError::Unavailable("down".into())), sec: Ok(vec![dated(Regulator::Sec, "b:1", "2026-05-01")]) };
    let out = d::gather(&r, &on("X", "TSX"), Some(&on("X", "NYSE")), "2026-09-16", 200);
    assert_eq!(ids(&out), ["b:1"]);
    assert!(!out.sources[&Regulator::Sedar].available);
    assert!(out.sources[&Regulator::Sedar].error.contains("down"));
}

#[test]
fn test_a_listing_no_regulator_covers_gathers_nothing() {
    let r = Regs { sedar: Ok(vec![dated(Regulator::Sedar, "a:1", "2026-01-01")]), sec: Ok(vec![]) };
    let out = d::gather(&r, &ListingKey::coin("BTC").unwrap(), None, "2026-09-16", 200);
    assert!(out.items.is_empty() && out.sources.is_empty());
}

// --- ContentResolutionTest

#[test]
fn test_it_picks_the_largest_substantive_document() {
    let base = "https://www.sec.gov/Archives/edgar/data/2106613/000110465926097327";
    let index = |_: &str| -> Fetched<Value> {
        Ok(json!({"directory": {"item": [
            {"name": "0001104659-26-097327-index.html", "size": 3000},
            {"name": "0001104659-26-097327.txt", "size": 106958},
            {"name": "tm2623033d1_6k.htm", "size": 1230},
            {"name": "tm2623033d1_ex99-1.htm", "size": 62339},
            {"name": "tm2623033d1_ex99-3.htm", "size": 1223},
        ]}}))
    };
    let seen = RefCell::new(None::<String>);
    let d = |row: &FiledDocument| -> Fetched<(Vec<u8>, String)> {
        seen.borrow_mut().get_or_insert(row.url.clone());
        Ok((b"<html>MD&A</html>".to_vec(), "text/html".into()))
    };
    edgar::content_with(&doc(Regulator::Sec, "", &format!("{}/tm2623033d1_6k.htm", base)), &index, &d).unwrap();
    assert!(seen.borrow().as_ref().unwrap().ends_with("tm2623033d1_ex99-1.htm"));
}

#[test]
fn test_it_falls_back_to_the_primary_when_the_index_is_unavailable() {
    let index = |_: &str| -> Fetched<Value> { Err(SourceError::Unavailable("no index".into())) };
    let called = RefCell::new(None::<String>);
    let d = |row: &FiledDocument| -> Fetched<(Vec<u8>, String)> {
        called.borrow_mut().get_or_insert(row.url.clone());
        Ok((b"x".to_vec(), "text/html".into()))
    };
    let _ = edgar::content_with(&doc(Regulator::Sec, "", "https://www.sec.gov/Archives/edgar/data/1/2/primary.htm"), &index, &d);
    assert!(called.borrow().as_ref().unwrap().ends_with("primary.htm"));
}

// --- OwnershipEnrichmentTest

#[test]
fn test_13g_yields_a_stake_title_and_summary() {
    let xml = "<edgarSubmission><submissionType>SCHEDULE 13G/A</submissionType>\
               <issuerName>Quantum eMotion Corp.</issuerName>\
               <reportingPersonName>Capital Ventures International</reportingPersonName>\
               <reportingPersonName>Susquehanna Advisors Group, Inc.</reportingPersonName>\
               <classPercent>2.4</classPercent></edgarSubmission>";
    let seen = RefCell::new(None::<String>);
    let d = |row: &FiledDocument| -> Fetched<(Vec<u8>, String)> {
        seen.borrow_mut().get_or_insert(row.url.clone());
        Ok((xml.as_bytes().to_vec(), "application/xml".into()))
    };
    let out = edgar::enrichment_with(
        &doc(Regulator::Sec, "SCHEDULE 13G/A", "https://www.sec.gov/Archives/edgar/data/1/2/xslSCHEDULE_13G_X02/primary_doc.xml"),
        &d,
    )
    .unwrap();
    assert!(!seen.borrow().as_ref().unwrap().contains("xsl"));
    assert!(out.subject.contains("2.4%"));
    assert!(out.subject.contains("Capital Ventures International"));
    assert!(out.summary.contains("2.4% of Quantum eMotion Corp."));
}

#[test]
fn test_non_ownership_forms_defer_to_the_model() {
    assert!(edgar::enrichment(&doc(Regulator::Sec, "6-K", "https://www.sec.gov/x/y.htm")).is_none());
}

// --- CategoryMappingTest

#[test]
fn test_offering_forms_and_administrative_f_forms() {
    assert_eq!(edgar::category("F-1"), d::OFFERINGS);
    assert_eq!(edgar::category("F-10"), d::OFFERINGS);
    assert_eq!(edgar::category("S-1"), d::OFFERINGS);
    assert_eq!(edgar::category("424B5"), d::OFFERINGS);
    assert_eq!(edgar::category("F-X"), d::OTHER);
    assert_eq!(edgar::category("F-N"), d::OTHER);
    assert_eq!(edgar::category("25"), d::EVENTS);
    assert_eq!(edgar::category("6-K"), d::FINANCIALS);
    assert_eq!(edgar::category("SCHEDULE 13G"), d::INSIDER);
}
