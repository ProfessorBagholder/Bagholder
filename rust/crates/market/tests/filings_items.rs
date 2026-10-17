//! The disclosures pipeline, pinned end to end: SEDAR+'s result pages and
//! EDGAR's submissions parsed into documents, what each source's own rules
//! read from a row without a reading (category, title, enrichment), the merge
//! of every source with its outcome, and what the store keeps, keeps of a
//! reading across a refresh, and reads back. The answers are held in
//! `golden/filings_items.json`, so a change of representation must leave every
//! document as it was.
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-market --test filings_items`,
//! and read the diff.

use bagholder_core::ListingKey;
use bagholder_market::disclosures::{self as d, Fetched, Regulators, SourceError};
use bagholder_market::{edgar, sedar};
use bagholder_store::feeds::{self as sf, FiledDocument, Filer, Regulator};
use serde_json::{json, Map, Value};

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

fn fixture(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures").join(name);
    std::fs::read_to_string(p).unwrap()
}

fn submissions() -> Value {
    json!({
        "name": "NVIDIA CORP",
        "filings": {"recent": {
            "form": ["10-Q", "8-K", "4", "SCHEDULE 13D/A", "424B5", "DEF 14A", "NT 10-K", "144", "6-K"],
            "filingDate": ["2026-08-05", "2026-08-01", "2026-07-30", "2026-07-20", "2026-07-10", "2026-06-15", "2026-06-01", "2026-05-20", "2026-05-02"],
            "primaryDocument": ["nvda-10q.htm", "nvda-8k.htm", "form4.xml", "sc13da.htm", "424b5.htm", "proxy.htm", "", "primary_doc.xml", "6k.htm"],
            "accessionNumber": ["0001-26-01", "0001-26-02", "0001-26-03", "0001-26-04", "0001-26-05", "0001-26-06", "0001-26-07", "0001-26-08", "0001-26-09"],
            "primaryDocDescription": ["", "Current report", "", "", "", "", "", "", "Report of foreign issuer"],
        }},
    })
}

/// What one regulator answers in a case: who files the listing (or why no one is
/// named, or that it could not be asked), and the filer's documents (or a failure).
#[derive(Clone)]
struct Answers {
    filer: Fetched<Result<String, String>>,
    documents: Fetched<Vec<FiledDocument>>,
}

/// The two regulators standing in: the SEC for a US listing, SEDAR+ for a Canadian one.
struct Regs {
    sedar: Answers,
    sec: Answers,
}

impl Regulators for Regs {
    fn filer(&self, listing: &ListingKey, _today: &str) -> Option<(Regulator, Fetched<Result<String, String>>)> {
        if listing.us() {
            Some((Regulator::Sec, self.sec.filer.clone()))
        } else if listing.canadian() {
            Some((Regulator::Sedar, self.sedar.filer.clone()))
        } else {
            None
        }
    }
    fn documents(&self, filer: &Filer, _limit: usize) -> Fetched<Vec<FiledDocument>> {
        match filer.source {
            Regulator::Sec => self.sec.documents.clone(),
            Regulator::Sedar => self.sedar.documents.clone(),
        }
    }
    fn interlisted(&self) -> Fetched<(String, Vec<bagholder_store::feeds::Interlisted>)> {
        Ok(("2026-10-01".into(), vec![]))
    }
}

fn answers() -> Value {
    let mut out = Map::new();

    // SEDAR+: the result page, each row as a document, the issuer directory
    let raw = sedar::parse_filings(&fixture("search_documents.html"));
    out.insert("sedar_rows".into(), cell(&raw));
    let sedar_items: Vec<_> = raw.iter().map(|r| sedar::to_item(r, "000012345")).collect();
    out.insert("sedar_items".into(), cell(&sedar_items));
    out.insert("sedar_issuers".into(), cell(&sedar::parse_reporting_issuers(&fixture("reporting_issuers.html"))));

    // EDGAR: the submissions as documents
    let subs = |_: &str| -> Fetched<Value> { Ok(submissions()) };
    let sec_items = edgar::fetch_with(1045810, 200, &subs).unwrap();
    out.insert("sec_items".into(), cell(&sec_items));

    // what a row says of itself without a reading
    // EDGAR's enrichment of a form it reads fetches the document: here it is not reached
    let offline = |_: &_| -> Fetched<(Vec<u8>, String)> { Err(SourceError::Other("offline".into())) };
    let read: Vec<Value> = sedar_items
        .iter()
        .map(|r| json!({"category": cell(&d::categorize(r)), "quick": d::quick_title(r), "enrichment": cell(&d::enrichment(r))}))
        .chain(sec_items.iter().map(|r| json!({"category": cell(&d::categorize(r)), "quick": d::quick_title(r), "enrichment": cell(&edgar::enrichment_with(r, &offline))})))
        .collect();
    out.insert("read_without_reading".into(), Value::Array(read));

    // EDGAR's own reading of an ownership form
    let xml = "<edgarSubmission><submissionType>SCHEDULE 13G/A</submissionType>\
               <issuerName>Quantum eMotion Corp.</issuerName>\
               <reportingPersonName>Capital Ventures International</reportingPersonName>\
               <classPercent>2.4</classPercent></edgarSubmission>";
    let doc = |_: &_| -> Fetched<(Vec<u8>, String)> { Ok((xml.as_bytes().to_vec(), "application/xml".into())) };
    let g13 = sec_items.iter().find(|r| r.form == "SCHEDULE 13D/A").unwrap();
    out.insert("sec_ownership_enrichment".into(), cell(&edgar::enrichment_with(g13, &doc)));

    // a listing's own regulator, and its interlisted partner's, merged, each with its outcome
    let ok = |id: &str, docs: &[FiledDocument]| Answers { filer: Ok(Ok(id.into())), documents: Ok(docs.to_vec()) };
    let us = ListingKey::on("NVDA", "NASDAQ").unwrap();
    let ca = ListingKey::on("NVDA", "TSX").unwrap();
    let both = Regs { sedar: ok("000012345", &sedar_items), sec: ok("1045810", &sec_items) };
    let down = Regs { sedar: Answers { filer: Ok(Ok("000012345".into())), documents: Err(SourceError::Unavailable("SEDAR+ is unavailable.".into())) }, sec: ok("1045810", &[]) };
    let named_no_one = Regs { sedar: Answers { filer: Ok(Err("SEDAR+ lists no single issuer named NVIDIA.".into())), documents: Ok(vec![]) }, sec: Answers { filer: Err(SourceError::Unavailable("sec.gov is resting".into())), documents: Ok(vec![]) } };
    out.insert("merge_both".into(), cell(&d::gather(&both, &us, Some(&ca), "2026-09-22", 12)));
    out.insert("merge_down".into(), cell(&d::gather(&down, &us, Some(&ca), "2026-09-22", 50)));
    out.insert("merge_error".into(), cell(&d::gather(&named_no_one, &us, Some(&ca), "2026-09-22", 50)));
    // a listing the interlisted list pairs with none shows its own regulator's alone
    out.insert("merge_unpaired".into(), cell(&d::gather(&both, &ListingKey::on("QNC", "TSX-V").unwrap(), None, "2026-09-22", 3)));

    // the store: both sources, a reading, a refresh that keeps it, a source dropping a row
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&conn).unwrap();
    conn.execute_batch(bagholder_sources::cache::MIGRATIONS[5].sql).unwrap();
    const NOW: &str = "2026-09-22T15:00:00Z";
    const LATER: &str = "2026-09-23T15:00:00Z";
    let sedar_filer = Filer { source: Regulator::Sedar, id: "000012345".into() };
    let sec_filer = Filer { source: Regulator::Sec, id: "1045810".into() };
    let filers = [sedar_filer.clone(), sec_filer.clone()];
    sf::replace_filings(&conn, &sedar_filer, &sedar_items, NOW).unwrap();
    sf::replace_filings(&conn, &sec_filer, &sec_items, NOW).unwrap();
    let first = cell(&sf::filings_of(&conn, &filers).unwrap());
    let sec0 = sec_items[0].id.clone();
    let sec1 = sec_items[1].id.clone();
    let sedar0 = sedar_items[0].id.clone();
    sf::set_filing_enrichment(&conn, Regulator::Sec, &sec0, Some("Quarterly report"), Some("Revenue rose."), Some(7), None, NOW).unwrap();
    sf::set_filing_enrichment(&conn, Regulator::Sec, &sec1, Some("Current report"), None, None, Some(true), NOW).unwrap();
    sf::set_filing_enrichment(&conn, Regulator::Sedar, &sedar0, None, Some("An interim report."), Some(3), Some(false), NOW).unwrap();
    let read = cell(&sf::filings_of(&conn, &filers).unwrap());
    sf::replace_filings(&conn, &sec_filer, &sec_items[..sec_items.len() - 1], LATER).unwrap();
    out.insert("store_first".into(), first);
    out.insert("store_read".into(), read);
    out.insert("store_refreshed".into(), cell(&sf::filings_of(&conn, &filers).unwrap()));
    let all: Vec<(String, Vec<sf::Filing>)> = sf::filings_all(&conn).unwrap().into_iter().map(|(f, rows)| (format!("{}:{}", f.source.as_str(), f.id), rows)).collect();
    out.insert("store_all".into(), cell(&all));
    out.insert("store_one".into(), cell(&sf::filing(&conn, Regulator::Sec, &sec0).unwrap()));
    out.insert("store_none".into(), cell(&sf::filing(&conn, Regulator::Sec, "sec:nope").unwrap()));
    Value::Object(out)
}

#[test]
fn test_every_filing_is_what_it_was() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/filings_items.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/filings_items.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}
