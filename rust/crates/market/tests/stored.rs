//! What the readers keep in the market cache, on the cache's own migrations:
//! a counter moves when its rows change and only then (docs/architecture.md,
//! rules 3 and 4), an item reads back as stored, and a stored row whose JSON does
//! not parse is an error of the read, never an empty value.

use bagholder_market::stored::feeds::{self, ExposureRecord, Feed, Gauge, NewsItem, NewsKind, ShortPoint, Shorts, Weights};
use bagholder_market::stored::{gens, rows};

fn db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    for m in &bagholder_sources::cache::MIGRATIONS {
        conn.execute_batch(m.sql).unwrap();
    }
    conn
}

fn gen(conn: &rusqlite::Connection, name: &str) -> i64 {
    *gens::all(conn).unwrap().get(name).unwrap_or_else(|| panic!("no counter named {name}"))
}

const NOW: &str = "2026-09-26T12:00:00Z";

#[test]
fn test_the_counters_named_are_the_ones_the_cache_keeps() {
    let conn = db();
    let kept: Vec<String> = gens::all(&conn).unwrap().into_keys().collect();
    assert_eq!(kept, gens::names(), "the cache's migrations make these counters and no others");
    let triggered: Vec<String> = conn
        .prepare("SELECT DISTINCT substr(name, 5, length(name) - 8) FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'gen\\_%\\_ins' ESCAPE '\\' ORDER BY 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(triggered, gens::names(), "each counter has its table's triggers");
}

#[test]
fn test_news_and_a_universe_read_again_unchanged_are_not_a_change() {
    let conn = db();
    let story = [NewsItem {
        id: "n1".into(), headline: "QNC files its quarter".into(), source: "Newswire".into(), url: "https://example.test/n1".into(),
        published_at: "2026-09-19T12:00:00Z".into(), summary: String::new(), kind: NewsKind::Story, via: Feed::Tmx,
    }];
    let qnc = bagholder_core::ListingKey::on("QNC", "TSX-V").unwrap();
    feeds::replace_news(&conn, &qnc, "TSX-V", &story, "2026-09-20T10:00:00Z").unwrap();
    let n = gen(&conn, "news");
    feeds::replace_news(&conn, &qnc, "TSX-V", &story, "2026-09-20T10:15:00Z").unwrap();
    assert_eq!(gen(&conn, "news"), n);

    let rows = [bagholder_model::input::UniverseRow { symbol: "AAA".into(), name: "Aaa Corp".into(), value: 10.0, percent_change: Some(1.5), sector: "Energy".into(), country: "CA".into() }];
    feeds::replace_universe(&conn, "tsx60", &rows, "2026-09-20T10:00:00Z").unwrap();
    let u = gen(&conn, "universes");
    feeds::replace_universe(&conn, "tsx60", &rows, "2026-09-20T10:30:00Z").unwrap();
    assert_eq!(gen(&conn, "universes"), u);
    let mut ticked = rows.clone();
    ticked[0].percent_change = Some(1.6);
    feeds::replace_universe(&conn, "tsx60", &ticked, "2026-09-20T11:00:00Z").unwrap();
    assert!(gen(&conn, "universes") > u);
}

const ALL: [Feed; 7] = [Feed::Tmx, Feed::TmxMedia, Feed::Nasdaq, Feed::NasdaqPress, Feed::Yahoo, Feed::Sa, Feed::Gnews];

#[test]
fn test_every_feed_parses_back_to_itself() {
    for f in ALL {
        assert_eq!(Feed::parse(f.as_str()), Some(f), "{}", f.as_str());
    }
    assert_eq!(Feed::parse("nonsense"), None);
}

#[test]
fn test_of_id_reads_the_feed_a_prefix_names() {
    assert_eq!(Feed::of_id("tmx:10"), Some(Feed::Tmx));
    assert_eq!(Feed::of_id("nasdaq:1"), Some(Feed::Nasdaq));
    assert_eq!(Feed::of_id("yahoo:a1"), Some(Feed::Yahoo));
    assert_eq!(Feed::of_id("sa:1"), Some(Feed::Sa));
    assert_eq!(Feed::of_id("gnews:1"), Some(Feed::Gnews));
    assert_eq!(Feed::of_id("unknown:1"), None);
    assert_eq!(Feed::of_id("nothing-here"), None);
}

#[test]
fn test_a_replaced_item_reads_back_the_same_through_its_feed() {
    let conn = db();
    let item = NewsItem {
        id: "tmx:10".into(),
        headline: "Zqx Closes $40M Financing Round".into(),
        source: "Newswire".into(),
        url: "https://example.test/news/10".into(),
        published_at: "2026-09-05T12:00:00Z".into(),
        summary: "Zqx closed its bought deal.".into(),
        kind: NewsKind::Release,
        via: Feed::Tmx,
    };
    let zqx = bagholder_core::ListingKey::on("ZQX", "TSX-V").unwrap();
    feeds::replace_news(&conn, &zqx, "TSX-V", &[item.clone()], "2026-09-05T12:05:00Z").unwrap();
    let stored = feeds::news_for(&conn, &zqx).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].feed, Some(Feed::Tmx));
    assert_eq!(stored[0].item(), Some(item));
    assert_eq!(rows::news(&conn).unwrap().len(), 1);
}

#[test]
fn test_an_exposure_whose_stored_weights_do_not_parse_is_an_error_not_an_empty_record() {
    let c = db();
    let rec = ExposureRecord { sectors: Weights(vec![("Energy".into(), 0.5)]), coverage: 0.5, source: "s".into(), ..Default::default() };
    feeds::replace_exposure(&c, "sec-1", &rec, NOW).unwrap();
    assert_eq!(feeds::exposure_record(&c, "sec-1").unwrap().unwrap().record, rec);
    assert_eq!(rows::exposures(&c).unwrap()["sec-1"].sectors, vec![("Energy".to_string(), 0.5)]);
    c.execute("UPDATE exposures SET sectors = '{not json' WHERE key = 'sec-1'", []).unwrap();
    let e = feeds::exposure_record(&c, "sec-1").unwrap_err().to_string();
    assert!(e.contains("sectors"), "{e}");
    assert!(rows::exposures(&c).is_err(), "the model's read fails too, rather than reading the security as holding nothing");
    // an empty column is a record with no weights
    c.execute("UPDATE exposures SET sectors = '' WHERE key = 'sec-1'", []).unwrap();
    assert!(feeds::exposure_record(&c, "sec-1").unwrap().unwrap().record.sectors.is_empty());
}

#[test]
fn test_short_selling_whose_stored_series_does_not_parse_is_an_error_on_read_and_on_save() {
    let c = db();
    let qnc = bagholder_core::ListingKey::on("QNC", "TSXV").unwrap();
    let rec = Shorts { symbol: "QNC".into(), exchange: "TSXV".into(), series: Some(vec![ShortPoint { date: "2026-09-15".into(), shares: 10.0 }]), ..Default::default() };
    feeds::save_shorts(&c, &qnc, &rec, NOW, 1).unwrap();
    assert!(feeds::shorts_for(&c, &qnc).unwrap().is_some());
    c.execute("UPDATE shorts SET series = '[{' WHERE listing = ?", [qnc.to_string()]).unwrap();
    assert!(feeds::shorts_for(&c, &qnc).unwrap_err().to_string().contains("series"));
    // a later read that did not ask for the run keeps the stored one: a stored run it cannot read fails it
    let later = Shorts { series: None, ..rec.clone() };
    assert!(feeds::save_shorts(&c, &qnc, &later, NOW, 1).is_err());
    // with no row stored yet there is nothing to keep, and nothing fails
    let other = Shorts { symbol: "ABC".into(), series: None, ..rec };
    feeds::save_shorts(&c, &bagholder_core::ListingKey::on("ABC", "TSXV").unwrap(), &other, NOW, 1).unwrap();
}

#[test]
fn test_a_gauge_whose_stored_payload_does_not_parse_is_an_error() {
    let c = db();
    let g = Gauge { index: "stocks".into(), source: "cnn".into(), score: 40.0, rating: "fear".into(), as_of: NOW.into(), previous: vec![], parts: vec![], series: vec![] };
    feeds::save_gauge(&c, "stocks", &g, NOW, 1).unwrap();
    assert_eq!(feeds::gauge(&c, "stocks").unwrap().unwrap().gauge.score, 40.0);
    c.execute("UPDATE gauges SET payload = 'nope' WHERE name = 'stocks'", []).unwrap();
    assert!(feeds::gauge(&c, "stocks").unwrap_err().to_string().contains("payload"));
}

#[test]
fn test_a_filing_column_holding_a_number_reads_as_its_text() {
    let c = db();
    let e = String::new;
    let doc = feeds::FiledDocument { id: "d1".into(), source: feeds::Regulator::Sedar, category: e(), profile_no: e(), issuer: e(), form: "News release".into(), title: e(), date: "2026-09-01".into(), date_text: e(), size: e(), url: e() };
    let filer = feeds::Filer { source: feeds::Regulator::Sedar, id: "000012345".into() };
    feeds::replace_filings(&c, &filer, &[doc], NOW).unwrap();
    c.execute("UPDATE filings SET size = 1234 WHERE id = 'd1'", []).unwrap();
    assert_eq!(feeds::filings_of(&c, std::slice::from_ref(&filer)).unwrap()[0].doc.size, "1234");
    c.execute("UPDATE filings SET size = x'00' WHERE id = 'd1'", []).unwrap();
    assert!(feeds::filings_of(&c, std::slice::from_ref(&filer)).is_err());
}
