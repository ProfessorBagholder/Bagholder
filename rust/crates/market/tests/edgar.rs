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

fn set_up() {
    edgar::set_ticker_map(edgar::parse_tickers(&ticker_list()).unwrap());
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

/// A US listing's filer is the SEC's own for its ticker on a US exchange; a
/// listing on another market is never matched to a US filer by its ticker.
#[test]
fn test_a_us_listing_is_its_secs_filer_and_no_other_market_is_matched_by_ticker() {
    set_up();
    assert_eq!(edgar::filer_of(&on("NVDA", "NASDAQ")).unwrap().map(|f| f.0), Some(1045810));
    assert_eq!(edgar::filer_of(&on("BRK.B", "NYSE")).unwrap().map(|f| f.0), Some(1067983));
    assert_eq!(edgar::filer_of(&on("SHOP", "TSX")).unwrap(), None, "the TSX listing reaches the SEC only through the interlisted list");
    assert_eq!(edgar::filer_of(&on("ZZZZ", "NASDAQ")).unwrap(), None);
    assert_eq!(edgar::filer_of(&on("OTCX", "NYSE")).unwrap(), None, "an over-the-counter row is no listing on a US exchange");
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
