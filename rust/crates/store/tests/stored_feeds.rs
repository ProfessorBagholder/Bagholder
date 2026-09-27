//! What the feeds stored reads back as stored, and a stored row whose JSON
//! does not parse is an error of the read, never an empty value.

use bagholder_store::feeds::{self, ExposureRecord, Gauge, Shorts, ShortPoint, Weights};
use bagholder_store::{open_db, relabel};

fn db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_db(&dir.path().join("bagholder.db")).unwrap();
    relabel::ensure(&conn).unwrap();
    (dir, conn)
}

const NOW: &str = "2026-09-26T12:00:00Z";

#[test]
fn test_an_exposure_whose_stored_weights_do_not_parse_is_an_error_not_an_empty_record() {
    let (_d, c) = db();
    let rec = ExposureRecord { sectors: Weights(vec![("Energy".into(), 0.5)]), coverage: 0.5, source: "s".into(), ..Default::default() };
    feeds::replace_exposure(&c, "sec-1", &rec, NOW).unwrap();
    assert_eq!(feeds::exposure_record(&c, "sec-1").unwrap().unwrap().record, rec);
    c.execute("UPDATE exposures SET sectors = '{not json' WHERE key = 'sec-1'", []).unwrap();
    let e = feeds::exposure_record(&c, "sec-1").unwrap_err().to_string();
    assert!(e.contains("sectors"), "{e}");
    // an empty column is a record with no weights
    c.execute("UPDATE exposures SET sectors = '' WHERE key = 'sec-1'", []).unwrap();
    assert!(feeds::exposure_record(&c, "sec-1").unwrap().unwrap().record.sectors.is_empty());
}

#[test]
fn test_short_selling_whose_stored_series_does_not_parse_is_an_error_on_read_and_on_save() {
    let (_d, c) = db();
    let rec = Shorts { symbol: "QNC".into(), exchange: "TSXV".into(), series: Some(vec![ShortPoint { date: "2026-09-15".into(), shares: 10.0 }]), ..Default::default() };
    feeds::save_shorts(&c, &rec, NOW, 1).unwrap();
    assert!(feeds::shorts_for(&c, "QNC", "TSXV").unwrap().is_some());
    c.execute("UPDATE shorts SET series = '[{' WHERE symbol = 'QNC'", []).unwrap();
    assert!(feeds::shorts_for(&c, "QNC", "TSXV").unwrap_err().to_string().contains("series"));
    // a later read that did not ask for the run keeps the stored one: a stored run it cannot read fails it
    let later = Shorts { series: None, ..rec.clone() };
    assert!(feeds::save_shorts(&c, &later, NOW, 1).is_err());
    // with no row stored yet there is nothing to keep, and nothing fails
    let other = Shorts { symbol: "ABC".into(), series: None, ..rec };
    feeds::save_shorts(&c, &other, NOW, 1).unwrap();
}

#[test]
fn test_a_gauge_whose_stored_payload_does_not_parse_is_an_error() {
    let (_d, c) = db();
    let g = Gauge { index: "stocks".into(), source: "cnn".into(), score: 40.0, rating: "fear".into(), as_of: NOW.into(), previous: vec![], parts: vec![], series: vec![] };
    feeds::save_gauge(&c, "stocks", &g, NOW, 1).unwrap();
    assert_eq!(feeds::gauge(&c, "stocks").unwrap().unwrap().gauge.score, 40.0);
    c.execute("UPDATE gauges SET payload = 'nope' WHERE name = 'stocks'", []).unwrap();
    assert!(feeds::gauge(&c, "stocks").unwrap_err().to_string().contains("payload"));
}

#[test]
fn test_a_notification_whose_stored_extra_does_not_parse_is_an_error() {
    let (_d, c) = db();
    let n = feeds::add_notification(&c, "fills", "k", "t", "b", None, false, NOW).unwrap().unwrap();
    c.execute("UPDATE notifications SET extra = '{' WHERE id = ?", [n.id]).unwrap();
    assert!(feeds::list_notifications(&c, 0, "", false, 10, false).unwrap_err().to_string().contains("extra"));
}

#[test]
fn test_a_legacy_filing_column_holding_a_number_reads_as_its_text() {
    let (_d, c) = db();
    let e = String::new;
    let doc = feeds::FiledDocument { id: "d1".into(), source: feeds::Regulator::Sedar, category: e(), profile_no: e(), issuer: e(), form: "News release".into(), title: e(), date: "2026-09-01".into(), date_text: e(), size: e(), url: e() };
    feeds::replace_filings(&c, "QNC", feeds::Regulator::Sedar, &[doc], NOW).unwrap();
    c.execute("UPDATE filings SET size = 1234 WHERE id = 'd1'", []).unwrap();
    assert_eq!(feeds::filings_for(&c, "QNC").unwrap()[0].doc.size, "1234");
    c.execute("UPDATE filings SET size = x'00' WHERE id = 'd1'", []).unwrap();
    assert!(feeds::filings_for(&c, "QNC").is_err());
}
