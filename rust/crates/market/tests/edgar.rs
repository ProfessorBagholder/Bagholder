//! EDGAR. No network: the ticker map is primed and the
//! submissions, index and document requests are given as closures.
use bagholder_market::disclosures::{self as d, Fetched, Provider, SourceError};
use bagholder_market::edgar;
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

fn set_up() {
    let mut m = edgar::Tickers::new();
    m.insert("NVDA".into(), (1045810, "NVIDIA CORP".into()));
    m.insert("SHOP".into(), (1594805, "SHOPIFY INC.".into()));
    edgar::set_ticker_map(m);
}

fn subs(_: &str) -> Fetched<Value> {
    Ok(submissions())
}

fn fetch(symbol: &str, name: &str, exchange: &str, currency: &str) -> Vec<Value> {
    set_up();
    edgar::fetch_with(symbol, name, exchange, currency, 200, &subs).unwrap()
}

// --- EdgarUnitTest

#[test]
fn test_bare_ticker_strips_venue_and_dots() {
    assert_eq!(edgar::bare("SHOP.TO"), "SHOP");
    assert_eq!(edgar::bare("BRK.B"), "BRK-B");
    assert_eq!(edgar::bare("nvda"), "NVDA");
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

#[test]
fn test_covers_a_us_listing_and_a_known_ticker() {
    set_up();
    assert!(edgar::covers("NVDA", "NASDAQ", "USD"));
    assert!(edgar::covers("SHOP", "TSX", "CAD"), "a cross-listed ticker SEC knows");
    assert!(!edgar::covers("QNC", "TSX-V", "CAD"), "a pure-Canadian ticker SEC does not know");
}

#[test]
fn test_fetch_normalizes_rows_with_source_and_url() {
    let items = fetch("NVDA", "NVIDIA Corporation", "NASDAQ", "USD");
    assert_eq!(items.len(), 7);
    let first = &items[0];
    assert_eq!(first["source"], "SEC");
    assert_eq!(first["id"], "sec:0001-26-01");
    assert_eq!(first["type"], "10-Q");
    assert_eq!(first["category"], d::FINANCIALS);
    assert!(first["url"].as_str().unwrap().starts_with("https://www.sec.gov/Archives/edgar/data/1045810/000126"));
}

#[test]
fn test_a_missing_document_url_falls_back_to_the_company_page() {
    let items = fetch("NVDA", "", "NASDAQ", "USD");
    let nt = items.iter().find(|i| i["type"] == "NT 10-K").unwrap();
    assert!(nt["url"].as_str().unwrap().contains("browse-edgar"));
}

#[test]
fn test_name_guard_rejects_a_canadian_ticker_colliding_with_a_us_filer() {
    let items = fetch("NVDA", "Northvolt Canada Mining Corp.", "TSX-V", "CAD");
    assert!(items.is_empty(), "the SEC 'NVIDIA CORP' entity does not match the Canadian name");
}

#[test]
fn test_unknown_ticker_returns_empty() {
    assert!(fetch("ZZZZ", "", "NASDAQ", "USD").is_empty());
}

// --- NameMatchTest

#[test]
fn test_matches_ignore_corporate_suffixes_and_case() {
    assert!(d::names_match("Shopify Inc.", "SHOPIFY INC."));
    assert!(d::names_match("NVIDIA Corporation", "NVIDIA CORP"));
}

#[test]
fn test_unrelated_names_do_not_match() {
    assert!(!d::names_match("Quantum eMotion Corp.", "QUALCOMM INC"));
    assert!(!d::names_match("", "Anything"));
}

// --- DispatcherTest

struct Prov {
    source: &'static str,
    items: Vec<Value>,
    avail: bool,
    covers: bool,
    raises: Option<SourceError>,
}

impl Prov {
    fn new(source: &'static str, items: Vec<Value>) -> Prov {
        Prov { source, items, avail: true, covers: true, raises: None }
    }
}

impl Provider for Prov {
    fn source(&self) -> &str { self.source }
    fn available(&self) -> bool { self.avail }
    fn covers(&self, _: &str, _: &str, _: &str) -> bool { self.covers }
    fn fetch(&self, _: &str, _: &str, _: &str, _: &str, _: &str) -> Fetched<Vec<Value>> {
        match &self.raises { Some(e) => Err(e.clone()), None => Ok(self.items.clone()) }
    }
    fn document(&self, _: &Value) -> Fetched<(Vec<u8>, String)> {
        Ok((b"%PDF-".to_vec(), "application/pdf".into()))
    }
}

fn ids(out: &Value) -> Vec<String> {
    out["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn test_items_from_both_providers_merge_newest_first() {
    let a = Prov::new("A", vec![json!({"id": "a:1", "source": "A", "date": "2026-01-01"})]);
    let b = Prov::new("B", vec![json!({"id": "b:1", "source": "B", "date": "2026-05-01"})]);
    let out = d::fetch_from(&[&a, &b], "X", "", "", "", 200, "");
    assert_eq!(ids(&out), ["b:1", "a:1"]);
    assert_eq!(out["sources"]["A"]["matched"], true);
    assert_eq!(out["sources"]["B"]["matched"], true);
}

#[test]
fn test_a_failing_source_is_recorded_and_the_other_still_returns() {
    let mut a = Prov::new("A", vec![]);
    a.raises = Some(SourceError::Unavailable("down".into()));
    let b = Prov::new("B", vec![json!({"id": "b:1", "source": "B", "date": "2026-05-01"})]);
    let out = d::fetch_from(&[&a, &b], "X", "", "", "", 200, "");
    assert_eq!(ids(&out), ["b:1"]);
    assert_eq!(out["sources"]["A"]["available"], false);
    assert!(out["sources"]["A"]["error"].as_str().unwrap().contains("down"));
}

#[test]
fn test_a_source_that_does_not_cover_is_skipped() {
    let mut a = Prov::new("A", vec![json!({"id": "a:1", "source": "A", "date": "2026-01-01"})]);
    a.covers = false;
    let out = d::fetch_from(&[&a], "X", "", "", "", 200, "");
    assert_eq!(out["items"], json!([]));
    assert_eq!(out["sources"]["A"]["matched"], false);
}

#[test]
fn test_document_routes_to_the_rows_source() {
    let a = Prov::new("A", vec![]);
    let (_, ct) = d::document_from(&[&a], &json!({"source": "A", "id": "a:1"})).unwrap();
    assert_eq!(ct, "application/pdf");
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
    let doc = |row: &Value| -> Fetched<(Vec<u8>, String)> {
        seen.borrow_mut().get_or_insert(row["url"].as_str().unwrap().to_string());
        Ok((b"<html>MD&A</html>".to_vec(), "text/html".into()))
    };
    edgar::content_with(&json!({"url": format!("{}/tm2623033d1_6k.htm", base), "source": "SEC"}), &index, &doc).unwrap();
    assert!(seen.borrow().as_ref().unwrap().ends_with("tm2623033d1_ex99-1.htm"));
}

#[test]
fn test_it_falls_back_to_the_primary_when_the_index_is_unavailable() {
    let index = |_: &str| -> Fetched<Value> { Err(SourceError::Unavailable("no index".into())) };
    let called = RefCell::new(None::<String>);
    let doc = |row: &Value| -> Fetched<(Vec<u8>, String)> {
        called.borrow_mut().get_or_insert(row["url"].as_str().unwrap().to_string());
        Ok((b"x".to_vec(), "text/html".into()))
    };
    let _ = edgar::content_with(&json!({"url": "https://www.sec.gov/Archives/edgar/data/1/2/primary.htm", "source": "SEC"}), &index, &doc);
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
    let doc = |row: &Value| -> Fetched<(Vec<u8>, String)> {
        seen.borrow_mut().get_or_insert(row["url"].as_str().unwrap().to_string());
        Ok((xml.as_bytes().to_vec(), "application/xml".into()))
    };
    let out = edgar::enrichment_with(
        &json!({"type": "SCHEDULE 13G/A", "source": "SEC", "url": "https://www.sec.gov/Archives/edgar/data/1/2/xslSCHEDULE_13G_X02/primary_doc.xml"}),
        &doc,
    )
    .unwrap();
    assert!(!seen.borrow().as_ref().unwrap().contains("xsl"));
    let subject = out["subject"].as_str().unwrap();
    assert!(subject.contains("2.4%"));
    assert!(subject.contains("Capital Ventures International"));
    assert!(out["summary"].as_str().unwrap().contains("2.4% of Quantum eMotion Corp."));
}

#[test]
fn test_non_ownership_forms_defer_to_the_model() {
    assert!(edgar::enrichment(&json!({"type": "6-K", "source": "SEC", "url": "https://www.sec.gov/x/y.htm"})).is_none());
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
