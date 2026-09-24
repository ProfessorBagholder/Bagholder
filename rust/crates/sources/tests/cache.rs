//! The market cache: its schema as committed, a file of another store or a newer
//! build refused, and every table's typed write and read.

use std::time::Duration;

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, InstrumentId, Money, SourceName};
use bagholder_sources::cache::{MarketCache, OutcomeRow, StoredQuote, MIGRATIONS, SCHEMA};
use bagholder_sources::contract::{Benchmark, DataKind};
use bagholder_sources::health::{self, State};
use bagholder_sources::outcome::OutcomeKind;
use bagholder_sqlite::migrate;

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn id(n: u8) -> InstrumentId {
    InstrumentId::parse(&format!("0192a000-0000-7000-8000-0000000000{n:02}")).unwrap()
}

fn open() -> (tempfile::TempDir, MarketCache) {
    let dir = tempfile::tempdir().unwrap();
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", t("2026-09-24T00:00:00Z")).unwrap();
    (dir, cache)
}

#[test]
fn each_version_is_exactly_the_schema_committed_for_it() {
    for n in 1..=MIGRATIONS.len() {
        let dir = tempfile::tempdir().unwrap();
        let partial: &'static migrate::Schema = Box::leak(Box::new(migrate::Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..n] }));
        let (conn, _) = migrate::open(partial, &dir.path().join("market.db"), "test", t("2026-09-24T00:00:00Z")).unwrap();
        let text = migrate::schema_text(&conn).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("schema/v{n}.sql"));
        if std::env::var("BAGHOLDER_BLESS").is_ok() {
            std::fs::write(&path, &text).unwrap();
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} is not committed", path.display()));
        assert_eq!(text, committed, "schema {n} differs from {}: a released migration was edited", path.display());
    }
}

#[test]
fn a_book_is_never_opened_as_the_cache_nor_a_newer_cache_by_this_build() {
    let dir = tempfile::tempdir().unwrap();
    let book = dir.path().join("book.db");
    drop(bagholder_book::Book::open(&book, "test", t("2026-09-24T00:00:00Z")).unwrap());
    let before = std::fs::read(&book).unwrap();
    assert!(MarketCache::open(&book, "test", t("2026-09-24T00:00:00Z")).is_err());
    assert_eq!(std::fs::read(&book).unwrap(), before, "the book was touched");

    let path = dir.path().join("market.db");
    drop(MarketCache::open(&path, "test", t("2026-09-24T00:00:00Z")).unwrap());
    rusqlite::Connection::open(&path).unwrap().execute_batch("PRAGMA user_version = 99").unwrap();
    let err = MarketCache::open(&path, "test", t("2026-09-24T00:00:00Z")).err().expect("a newer file is refused");
    assert!(err.to_string().contains("99"), "{err}");
}

#[test]
fn a_quote_is_the_latest_per_source_and_reads_back_exactly() {
    let (_d, c) = open();
    let tmx = SourceName::named("tmx");
    let mut q = StoredQuote {
        instrument: id(1),
        source: tmx.clone(),
        price: Money::new(dec("218.31"), Currency::CAD),
        change: Some(dec("-0.40")),
        change_pct: None,
        quoted_at: t("2026-09-23T20:00:00Z"),
        allowance: Duration::ZERO,
        received_at: t("2026-09-23T20:01:00Z"),
    };
    c.store_quote(&q).unwrap();
    q.price = Money::new(dec("219"), Currency::CAD);
    q.quoted_at = t("2026-09-24T14:00:00Z");
    c.store_quote(&q).unwrap();
    let spot = StoredQuote { instrument: id(1), source: SourceName::named("coinbase-spot"), allowance: Duration::from_secs(60), ..q.clone() };
    c.store_quote(&spot).unwrap();
    let got = c.quotes().unwrap();
    assert_eq!(got.len(), 2);
    assert!(got.contains(&q) && got.contains(&spot));
}

#[test]
fn a_closed_day_is_written_once_and_a_later_different_value_is_kept_beside_it() {
    let (_d, c) = open();
    let yahoo = SourceName::named("yahoo");
    let first = c.store_closes(id(1), &[(d("2026-09-22"), dec("10.50")), (d("2026-09-23"), dec("10.75"))], Currency::USD, &yahoo, t("2026-09-23T21:00:00Z")).unwrap();
    assert!(first.is_empty());
    // the same again writes nothing; a different value for a day stands aside
    let again = c.store_closes(id(1), &[(d("2026-09-22"), dec("10.5")), (d("2026-09-23"), dec("10.80"))], Currency::USD, &yahoo, t("2026-09-24T21:00:00Z")).unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!((again[0].day, again[0].stands, again[0].later), (d("2026-09-23"), dec("10.75"), dec("10.80")));
    let closes = c.closes().unwrap();
    assert_eq!(closes[&id(1)][&d("2026-09-23")], Money::new(dec("10.75"), Currency::USD));
    assert_eq!(c.last_close_day(id(1)).unwrap(), Some(d("2026-09-23")));
    assert_eq!(c.close_days(id(1)).unwrap().len(), 2);
    let kept: i64 = c.connection().query_row("SELECT COUNT(*) FROM daily_closes", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, 3, "the later value is kept beside the first");
}

#[test]
fn a_benchmark_level_is_written_once() {
    let (_d, c) = open();
    let fred = SourceName::named("fred");
    c.store_benchmark(Benchmark::Sp500, &[(d("2026-09-22"), dec("6600.01"))], &fred, t("2026-09-23T12:00:00Z")).unwrap();
    let later = c.store_benchmark(Benchmark::Sp500, &[(d("2026-09-22"), dec("6600.02")), (d("2026-09-23"), dec("6610"))], &fred, t("2026-09-24T12:00:00Z")).unwrap();
    assert_eq!(later.len(), 1);
    let levels = c.benchmarks().unwrap();
    assert_eq!(levels[&Benchmark::Sp500][&d("2026-09-22")], dec("6600.01"));
    assert_eq!(c.benchmark_days(Benchmark::Sp500).unwrap().len(), 2);
}

#[test]
fn the_winner_is_remembered_per_instrument_and_kind() {
    let (_d, c) = open();
    assert_eq!(c.winner(id(1), DataKind::Quote).unwrap(), None);
    c.won(id(1), DataKind::Quote, &SourceName::named("tmx"), "QCN", t("2026-09-24T00:00:00Z")).unwrap();
    c.won(id(1), DataKind::Quote, &SourceName::named("tmx"), "QCN:US", t("2026-09-24T01:00:00Z")).unwrap();
    assert_eq!(c.winner(id(1), DataKind::Quote).unwrap(), Some((SourceName::named("tmx"), "QCN:US".to_string())));
    assert_eq!(c.winner(id(1), DataKind::DailyClose).unwrap(), None);
}

fn outcome(source: &'static str, kind: OutcomeKind, at: Timestamp) -> OutcomeRow {
    OutcomeRow { source: SourceName::named(source), host: "h".into(), kind: DataKind::Quote, instrument: Some(id(1)), outcome: kind, detail: "d".into(), shape_change: None, at }
}

#[test]
fn a_thousand_quote_outcomes_do_not_evict_the_banks_last() {
    let (_d, c) = open();
    c.record(&outcome("bank-of-canada", OutcomeKind::Answered, t("2026-09-01T21:00:00Z"))).unwrap();
    c.record(&outcome("tmx", OutcomeKind::Mismatch, t("2026-09-01T21:00:00Z"))).unwrap();
    let start = t("2026-09-02T00:00:00Z");
    for n in 0..1200 {
        c.record(&outcome("tmx", OutcomeKind::Answered, start + Duration::from_secs(n))).unwrap();
    }
    let bank = c.outcomes(&SourceName::named("bank-of-canada")).unwrap();
    assert_eq!(bank.len(), 1);
    let tmx = c.outcomes(&SourceName::named("tmx")).unwrap();
    // the newest thousand, and the newest of each kind (the one mismatch)
    assert_eq!(tmx.len(), 1001);
    assert_eq!(tmx[0].at, start + Duration::from_secs(1199));
    assert_eq!(health::last_of_each(&tmx)[&OutcomeKind::Mismatch].at, t("2026-09-01T21:00:00Z"));
    assert_eq!(health::state(&bank, t("2026-09-24T00:00:00Z")), State::Working);
    assert_eq!(c.sources().unwrap().len(), 2);
}
