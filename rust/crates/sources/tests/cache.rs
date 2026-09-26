//! The market cache: its schema as committed, a file of another store or a newer
//! build refused, and every table's typed write and read.

use std::time::Duration;

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, InstrumentId, Money, SourceName};
use bagholder_sources::cache::{ChainRead, MarketCache, OutcomeRow, ReadRow, StoredQuote, TrackerEvent, MIGRATIONS, SCHEMA};
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
fn a_trackers_close_and_events_are_written_once() {
    let (_d, c) = open();
    let yahoo = SourceName::named("yahoo");
    c.store_benchmark_closes(Benchmark::Sp500, &[(d("2026-09-22"), dec("660.01"))], &yahoo, t("2026-09-23T12:00:00Z")).unwrap();
    let later = c.store_benchmark_closes(Benchmark::Sp500, &[(d("2026-09-22"), dec("660.02")), (d("2026-09-23"), dec("661"))], &yahoo, t("2026-09-24T12:00:00Z")).unwrap();
    assert_eq!(later.len(), 1);
    let events = [(d("2026-09-19"), TrackerEvent::Dividend(dec("1.993"))), (d("2026-09-23"), TrackerEvent::Split { numerator: dec("2"), denominator: dec("1") })];
    assert!(c.store_benchmark_events(Benchmark::Sp500, &events, &yahoo, t("2026-09-24T12:00:00Z")).unwrap().is_empty());
    // the same again says nothing; a different dividend that day is kept beside it
    assert!(c.store_benchmark_events(Benchmark::Sp500, &events, &yahoo, t("2026-09-24T13:00:00Z")).unwrap().is_empty());
    let differs = c.store_benchmark_events(Benchmark::Sp500, &[(d("2026-09-19"), TrackerEvent::Dividend(dec("1.994")))], &yahoo, t("2026-09-24T14:00:00Z")).unwrap();
    assert_eq!((differs[0].kind, differs[0].stands.as_str(), differs[0].later.as_str()), ("dividend", "1.993", "1.994"));
    let series = c.benchmark_series().unwrap();
    let spy = &series[&Benchmark::Sp500];
    assert_eq!(spy.closes[&d("2026-09-22")], dec("660.01"));
    assert_eq!(spy.dividends[&d("2026-09-19")], dec("1.993"));
    assert_eq!(spy.splits[&d("2026-09-23")], (dec("2"), dec("1")));
    assert_eq!(c.benchmark_days(Benchmark::Sp500).unwrap().len(), 2);
}

#[test]
fn migration_3_clears_the_index_levels_their_reads_and_the_option_closes_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("market.db");
    let v2: &'static migrate::Schema = Box::leak(Box::new(migrate::Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..2] }));
    {
        let (conn, _) = migrate::open(v2, &path, "test", t("2026-09-24T00:00:00Z")).unwrap();
        conn.execute("INSERT INTO benchmarks (benchmark, day, level, source, first, received_at) VALUES ('SP500', '2026-09-22', '6600.01', 'fred', 1, '2026-09-23T12:00:00Z')", []).unwrap();
        for kind in ["benchmark", "option-close", "daily-close"] {
            conn.execute("INSERT INTO reads (subject, kind, source, first, last, outcome, at) VALUES ('s', ?1, 'x', '2026-09-01', '2026-09-22', 'answered', '2026-09-23T12:00:00Z')", [kind]).unwrap();
            conn.execute("INSERT INTO outcomes (source, host, kind, outcome, detail, at) VALUES ('x', 'h', ?1, 'answered', '', '2026-09-23T12:00:00Z')", [kind]).unwrap();
        }
    }
    let (c, _) = MarketCache::open(&path, "test", t("2026-09-24T01:00:00Z")).unwrap();
    assert!(c.benchmark_series().unwrap().is_empty());
    assert!(c.reads("s", DataKind::Benchmark).unwrap().is_empty());
    assert_eq!(c.reads("s", DataKind::DailyClose).unwrap().len(), 1, "other reads stay");
    let kinds: Vec<String> = c.connection().prepare("SELECT kind FROM reads UNION ALL SELECT kind FROM outcomes").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    assert!(!kinds.iter().any(|k| k == "option-close"), "{kinds:?}");
}

#[test]
fn the_chain_last_read_for_an_underlying_reads_back_exactly() {
    let (_d, c) = open();
    assert_eq!(c.option_chain("BBAI").unwrap(), None);
    let read = ChainRead { underlying: "BBAI".into(), session: d("2026-09-22"), made_at: t("2026-09-23T03:30:07Z"), last_modified: Some("Wed, 23 Sep 2026 03:30:10 GMT".into()), received_at: t("2026-09-23T04:00:00Z") };
    c.store_option_chain(&read).unwrap();
    assert_eq!(c.option_chain("BBAI").unwrap(), Some(read.clone()));
    let later = ChainRead { session: d("2026-09-23"), made_at: t("2026-09-23T20:40:00Z"), last_modified: None, ..read };
    c.store_option_chain(&later).unwrap();
    assert_eq!(c.option_chain("BBAI").unwrap(), Some(later));
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

#[test]
fn a_read_is_kept_per_span_newest_first_and_reads_back_exactly() {
    let (_d, c) = open();
    let r = |first: u8, last: u8, outcome: OutcomeKind, at: &str| ReadRow { source: SourceName::named("yahoo"), first: bagholder_core::jiff::civil::date(2026, 9, first as i8), last: bagholder_core::jiff::civil::date(2026, 9, last as i8), outcome, at: t(at) };
    c.store_read("x", DataKind::DailyClose, &r(1, 22, OutcomeKind::Unreachable, "2026-09-23T01:00:00Z")).unwrap();
    c.store_read("x", DataKind::DailyClose, &r(23, 23, OutcomeKind::Answered, "2026-09-23T22:00:00Z")).unwrap();
    // the same span again: the newest attempt replaces the older
    c.store_read("x", DataKind::DailyClose, &r(1, 22, OutcomeKind::Answered, "2026-09-23T21:00:00Z")).unwrap();
    assert_eq!(c.reads("x", DataKind::DailyClose).unwrap(), vec![r(23, 23, OutcomeKind::Answered, "2026-09-23T22:00:00Z"), r(1, 22, OutcomeKind::Answered, "2026-09-23T21:00:00Z")]);
    assert!(c.reads("x", DataKind::Benchmark).unwrap().is_empty());
    assert!(c.reads("y", DataKind::DailyClose).unwrap().is_empty());
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

#[test]
fn each_sources_newest_counted_outcome_is_the_one_that_says_how_it_is() {
    let (_d, c) = open();
    let at = |n: u64| t("2026-09-24T12:00:00Z") + Duration::from_secs(n);
    // every outcome that says how a source is, followed by one that does not
    for (n, kind) in OutcomeKind::ALL.into_iter().filter(|k| *k != OutcomeKind::NotCarried).enumerate() {
        let n = n as u64 * 10;
        c.record(&outcome("a", OutcomeKind::Answered, at(n))).unwrap();
        c.record(&outcome("a", kind, at(n + 1))).unwrap();
        c.record(&outcome("a", OutcomeKind::NotCarried, at(n + 2))).unwrap();
        c.record(&outcome("b", OutcomeKind::Unreachable, at(n + 3))).unwrap();
        let newest: Vec<_> = c.newest_counted().unwrap().into_iter().map(|o| (o.source.as_str().to_string(), o.outcome, o.at)).collect();
        assert_eq!(newest, vec![("a".to_string(), kind, at(n + 1)), ("b".to_string(), OutcomeKind::Unreachable, at(n + 3))]);
    }
    // a source that has only said it does not carry something says nothing of itself
    c.record(&outcome("c", OutcomeKind::NotCarried, at(1000))).unwrap();
    assert!(c.newest_counted().unwrap().iter().all(|o| o.source.as_str() != "c"));
}

#[test]
fn a_request_the_process_was_told_not_to_make_is_no_outcome_of_the_source() {
    let (_d, c) = open();
    let at = t("2026-09-24T12:00:00Z");
    // refused by BAGHOLDER_OFFLINE: nothing was asked, nothing is recorded
    let offline = OutcomeRow { detail: bagholder_net::client::OFFLINE.into(), ..outcome("a", OutcomeKind::Unreachable, at) };
    c.record(&offline).unwrap();
    // as it reads, or with the reader's own subject before it
    c.record(&OutcomeRow { detail: format!("SPY: {}", bagholder_net::client::OFFLINE), ..outcome("a", OutcomeKind::Unreachable, at) }).unwrap();
    assert!(c.outcomes(&SourceName::named("a")).unwrap().is_empty());
    assert!(c.newest_counted().unwrap().is_empty(), "a source nobody asked is not failing");
    // a source that could not be reached is still one, whatever else its words say
    c.record(&OutcomeRow { detail: "offline: the host said so".into(), ..outcome("a", OutcomeKind::Unreachable, at) }).unwrap();
    assert_eq!(c.newest_counted().unwrap().len(), 1);
}

#[test]
fn a_commit_to_the_cache_is_heard() {
    let (_d, c) = open();
    let heard = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let h = heard.clone();
    c.on_commit(std::sync::Arc::new(move || {
        h.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));
    c.record(&outcome("a", OutcomeKind::Unreachable, t("2026-09-24T12:00:00Z"))).unwrap();
    assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn a_source_failing_in_a_row_is_counted_to_its_last_answer() {
    let (_d, c) = open();
    let at = |n: u64| t("2026-09-24T12:00:00Z") + Duration::from_secs(n);
    let src = SourceName::named("a");
    c.record(&outcome("a", OutcomeKind::Answered, at(0))).unwrap();
    for n in 1..=3 {
        c.record(&outcome("a", OutcomeKind::Meaning, at(n))).unwrap();
    }
    // a "not carried" answer says nothing of the source and does not end the run
    c.record(&outcome("a", OutcomeKind::NotCarried, at(4))).unwrap();
    c.record(&outcome("a", OutcomeKind::Unreachable, at(5))).unwrap();
    assert_eq!(c.failures_in_a_row(&src, DataKind::Quote, None).unwrap(), 4);
    // another instrument's outcomes are its own
    assert_eq!(c.failures_in_a_row(&src, DataKind::Quote, Some(id(2))).unwrap(), 0);
    // another kind of read is its own
    assert_eq!(c.failures_in_a_row(&src, DataKind::Distributions, None).unwrap(), 0);
    c.record(&outcome("a", OutcomeKind::Answered, at(6))).unwrap();
    assert_eq!(c.failures_in_a_row(&src, DataKind::Quote, None).unwrap(), 0, "an answer ends the run");
}

#[test]
fn a_failing_source_rests_twice_as_long_each_time_up_to_six_hours() {
    use bagholder_sources::market::grown_rest;
    let base = Duration::from_secs(60);
    let rests: Vec<u64> = (0..=20).map(|n| grown_rest(base, n).as_secs()).collect();
    assert_eq!(&rests[..5], &[60, 60, 120, 240, 480], "its own rest after the first, doubling after");
    assert!(rests.windows(2).all(|w| w[0] <= w[1]), "never shorter after another failure");
    assert_eq!(*rests.last().unwrap(), 6 * 3600, "six hours at most");
    assert_eq!(grown_rest(Duration::from_secs(10 * 3600), 1).as_secs(), 6 * 3600, "a long own rest is capped too");
}
