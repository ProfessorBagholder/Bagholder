//! When closes and benchmark levels are read (on a clock handed in), and what a
//! run stores in the market cache from recorded replies.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bagholder_book::Book;
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, InstrumentId, Money, SourceName};
use bagholder_sources::outcome::OutcomeKind;
use std::time::Duration;
use bagholder_sources::cache::{MarketCache, ReadRow};
use bagholder_sources::contract::{Benchmark, DataKind, Listing, Market};
use bagholder_sources::market::{self, CloseState};
use bagholder_sources::needs::CloseNeed;
use bagholder_sources::read::Ctx;

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn eastern() -> TimeZone {
    TimeZone::get("America/Toronto").unwrap()
}

fn id(n: u8) -> InstrumentId {
    InstrumentId::parse(&format!("0192a000-0000-7000-8000-0000000000{n:02}")).unwrap()
}

fn listing(n: u8, kind: InstrumentKind, currency: Currency, symbol: &str, mic: Option<&str>) -> Listing {
    Listing { id: id(n), kind, currency, symbol: symbol.into(), venue_mic: mic.map(str::to_string), routes: BTreeMap::new() }
}

fn need(l: Listing, from: Date, to: Date) -> CloseNeed {
    CloseNeed { listing: l, from, to }
}

fn days(from: Date, to: Date) -> BTreeSet<Date> {
    let mut out = BTreeSet::new();
    let mut d = from;
    while d <= to {
        out.insert(d);
        d = d.tomorrow().unwrap();
    }
    out
}

fn read(first: Date, last: Date, outcome: OutcomeKind, at: &str) -> ReadRow {
    ReadRow { source: SourceName::named("yahoo"), first, last, outcome, at: t(at) }
}

const REST: Duration = Duration::from_secs(600);

#[test]
fn a_listings_close_is_due_once_its_session_has_settled() {
    let n = need(listing(1, InstrumentKind::Security, Currency::USD, "SPY", Some("ARCX")), date(2026, 9, 1), date(2026, 9, 23));
    let due = |state: &CloseState, now: &str| market::due_close(&n, Market::UnitedStates, state, t(now), &eastern(), REST);
    let none = CloseState::default();
    // 16:00 Eastern on the 23rd: the 22nd has settled, the 23rd has not
    assert_eq!(due(&none, "2026-09-23T20:00:00Z"), Some((date(2026, 9, 1), date(2026, 9, 22))));
    assert_eq!(due(&none, "2026-09-23T20:31:00Z"), Some((date(2026, 9, 1), date(2026, 9, 23))));
    // held to the 22nd by a read after it settled: only the 23rd, once it settles
    let held = CloseState { days: days(date(2026, 9, 1), date(2026, 9, 22)), reads: vec![read(date(2026, 9, 1), date(2026, 9, 22), OutcomeKind::Answered, "2026-09-22T21:00:00Z")] };
    assert_eq!(due(&held, "2026-09-23T20:00:00Z"), None);
    assert_eq!(due(&held, "2026-09-23T20:31:00Z"), Some((date(2026, 9, 23), date(2026, 9, 23))));
    // closes stored only from well after the first day held: the days before are due
    let late = CloseState { days: days(date(2026, 9, 15), date(2026, 9, 22)), reads: vec![read(date(2026, 9, 15), date(2026, 9, 22), OutcomeKind::Answered, "2026-09-22T21:00:00Z")] };
    assert_eq!(due(&late, "2026-09-23T20:00:00Z"), Some((date(2026, 9, 1), date(2026, 9, 14))));
    // a source that does not carry the listing is not asked again for those days
    let gone = CloseState { days: BTreeSet::new(), reads: vec![read(date(2026, 9, 1), date(2026, 9, 22), OutcomeKind::NotCarried, "2026-09-22T21:00:00Z")] };
    assert_eq!(due(&gone, "2026-09-23T20:31:00Z"), Some((date(2026, 9, 23), date(2026, 9, 23))));
    // a failure waits out its source's rest, then the same days are due again
    let failed = CloseState { days: BTreeSet::new(), reads: vec![read(date(2026, 9, 1), date(2026, 9, 22), OutcomeKind::Unreachable, "2026-09-23T20:00:00Z")] };
    assert_eq!(due(&failed, "2026-09-23T20:05:00Z"), None);
    assert_eq!(due(&failed, "2026-09-23T20:10:00Z"), Some((date(2026, 9, 1), date(2026, 9, 22))));
}

#[test]
fn a_holiday_is_asked_once_and_a_day_a_source_has_not_posted_stays_due() {
    let labour = need(listing(1, InstrumentKind::Security, Currency::USD, "SPY", Some("ARCX")), date(2026, 9, 1), date(2026, 9, 8));
    let due = |state: &CloseState, now: &str| market::due_close(&labour, Market::UnitedStates, state, t(now), &eastern(), REST);
    // asked the evening of Labour Day: the read held Friday the 4th last, so the
    // 7th is not settled by it, and is asked with the 8th once that settles
    let friday = CloseState { days: days(date(2026, 9, 1), date(2026, 9, 4)), reads: vec![read(date(2026, 9, 1), date(2026, 9, 4), OutcomeKind::Answered, "2026-09-07T21:00:00Z")] };
    assert_eq!(due(&friday, "2026-09-07T23:00:00Z"), Some((date(2026, 9, 7), date(2026, 9, 7))));
    // read again on the 8th after the close: it held the 8th, so the 7th is a day
    // with no session, and nothing is due
    let tuesday = CloseState { days: [date(2026, 9, 1), date(2026, 9, 2), date(2026, 9, 3), date(2026, 9, 4), date(2026, 9, 8)].into(), reads: vec![read(date(2026, 9, 7), date(2026, 9, 8), OutcomeKind::Answered, "2026-09-08T21:00:00Z"), friday.reads[0].clone()] };
    assert_eq!(due(&tuesday, "2026-09-08T23:00:00Z"), None);
}

#[test]
fn a_coins_day_settles_at_the_end_of_the_utc_day_every_day() {
    let n = need(listing(2, InstrumentKind::Crypto, Currency::CAD, "BTC", None), date(2026, 9, 19), date(2026, 9, 21));
    let none = CloseState::default();
    assert_eq!(market::due_close(&n, Market::Crypto, &none, t("2026-09-21T23:59:00Z"), &eastern(), REST), Some((date(2026, 9, 19), date(2026, 9, 20))));
    assert_eq!(market::due_close(&n, Market::Crypto, &none, t("2026-09-22T00:00:00Z"), &eastern(), REST), Some((date(2026, 9, 19), date(2026, 9, 21))));
    // the pairs: its own market first, then its USD market
    assert_eq!(market::coin_pairs(&n.listing), vec!["BTC-CAD".to_string(), "BTC-USD".to_string()]);
}

const YAHOO: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";
const TMX: &str = "https://app-money.tmx.com/graphql";

#[test]
fn a_run_stores_closes_and_levels_and_asks_again_only_when_due() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-24T04:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(
        common::Recorded::new()
            // ENB on the TSX, 2026-08-03 to 2026-09-23
            .with(&format!("{YAHOO}ENB.TO?period1=1785715200&period2=1790208000&interval=1d&events=div%7Csplit"), 200, "yahoo", "ENB.TO-2026-08-03-2026-09-24.json")
            // BTC in CAD: its own pair has no market; its USD market answers
            .with("https://api.exchange.coinbase.com/products/BTC-CAD/candles?granularity=86400&start=2026-09-01T00:00:00Z&end=2026-09-04T00:00:00Z", 404, "coinbase-exchange", "candles-BTC-CAD-status-404.json")
            .with("https://api.exchange.coinbase.com/products/BTC-USD/candles?granularity=86400&start=2026-09-01T00:00:00Z&end=2026-09-04T00:00:00Z", 200, "coinbase-exchange", "candles-BTC-USD-2026-09-01-2026-09-05.json")
            // the benchmarks: FRED, then ^GSPC before FRED's first day, then TMX's two series
            .with("https://fred.stlouisfed.org/graph/fredgraph.csv?id=SP500", 200, "fred", "SP500.csv")
            .with(&format!("{YAHOO}%5EGSPC?period1=1474675200&period2=1790294400&interval=1mo&events=split"), 200, "yahoo", "GSPC-splits-2016-09-24-2026-09-24.json")
            .with(&format!("{YAHOO}%5EGSPC?period1=1451865600&period2=1474675200&interval=1d&events=div%7Csplit"), 200, "yahoo", "GSPC-2016-01-04-2016-09-23.json")
            .with_body(TMX, "\"symbol\":\"^TSX\"", 200, "tmx", "series-TSX-2026-09-01-2026-09-23.json")
            // (the ^TSX reply stands in for ^TX60's here: the reader checks the days, not the index's name)
            .with_body(TMX, "\"symbol\":\"^TX60\"", 200, "tmx", "series-TSX-2026-09-01-2026-09-23.json"),
    );
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let zone = eastern();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    let enb = listing(1, InstrumentKind::Security, Currency::CAD, "ENB", Some("XTSE"));
    let btc = listing(2, InstrumentKind::Crypto, Currency::CAD, "BTC", None);
    let needs = [need(enb, date(2026, 8, 3), date(2026, 9, 24)), need(btc, date(2026, 9, 1), date(2026, 9, 4))];
    market::read_closes(&ctx, &needs).unwrap();
    let closes = cache.closes().unwrap();
    assert_eq!(closes[&id(1)][&date(2026, 8, 4)], Money::new(Dec::parse("75.08999633789062").unwrap(), Currency::CAD));
    assert_eq!(closes[&id(1)].keys().next_back(), Some(&date(2026, 9, 23)));
    // the coin's closes are its USD market's, as stated
    assert_eq!(closes[&id(2)][&date(2026, 9, 1)], Money::new(Dec::parse("77398.69").unwrap(), Currency::USD));
    assert_eq!(cache.winner(id(1), DataKind::DailyClose).unwrap().map(|w| w.1), Some("ENB.TO".to_string()));
    assert_eq!(cache.winner(id(2), DataKind::DailyClose).unwrap().map(|w| w.1), Some("BTC-USD".to_string()));

    market::read_benchmarks(&ctx, date(2016, 1, 4)).unwrap();
    let levels = cache.benchmarks().unwrap();
    assert_eq!(levels[&Benchmark::Sp500].get(&date(2016, 9, 26)), Some(&Dec::parse("2146.10").unwrap()));
    assert!(levels[&Benchmark::Sp500].contains_key(&date(2016, 1, 4)), "the days before FRED's window from ^GSPC");
    assert!(levels[&Benchmark::Tsx].contains_key(&date(2026, 9, 23)));
    assert!(levels[&Benchmark::Tx60].contains_key(&date(2026, 9, 23)));
    let asked = recorded.asked.lock().unwrap().len();
    // ^GSPC before FRED asks its later splits first: eight requests
    assert_eq!(asked, 8);

    // the same moment again: nothing is due
    market::read_closes(&ctx, &needs).unwrap();
    market::read_benchmarks(&ctx, date(2016, 1, 4)).unwrap();
    assert_eq!(recorded.asked.lock().unwrap().len(), asked);
}

#[test]
fn a_fill_on_alpha_is_asked_as_a_tsx_issue_then_a_venture_one_and_the_answer_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-24T04:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    // 01 Communique, filled on Alpha, is a TSX Venture issue
    let recorded = Arc::new(
        common::Recorded::new()
            .with(&format!("{YAHOO}ONE.TO?period1=1752278400&period2=1790294400&interval=1mo&events=split"), 404, "yahoo", "ONE.TO-splits-status-404.json")
            .with(&format!("{YAHOO}ONE.V?period1=1752278400&period2=1790294400&interval=1mo&events=split"), 200, "yahoo", "ONE.V-splits-2025-07-12-2026-09-24.json")
            .with(&format!("{YAHOO}ONE.V?period1=1749513600&period2=1752278400&interval=1d&events=div%7Csplit"), 200, "yahoo", "ONE.V-2025-06-10-2025-07-11.json"),
    );
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let zone = eastern();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    let one = need(listing(1, InstrumentKind::Security, Currency::CAD, "ONE", Some("XATS")), date(2025, 6, 10), date(2025, 7, 11));
    market::read_closes(&ctx, &[one]).unwrap();
    assert_eq!(
        *recorded.asked.lock().unwrap(),
        vec![
            format!("{YAHOO}ONE.TO?period1=1752278400&period2=1790294400&interval=1mo&events=split"),
            format!("{YAHOO}ONE.V?period1=1752278400&period2=1790294400&interval=1mo&events=split"),
            format!("{YAHOO}ONE.V?period1=1749513600&period2=1752278400&interval=1d&events=div%7Csplit"),
        ]
    );
    let closes = cache.closes().unwrap();
    assert!(closes[&id(1)].keys().next().is_some_and(|d| *d >= date(2025, 6, 10)));
    assert!(closes[&id(1)].values().all(|m| m.currency == Currency::CAD));
    assert_eq!(cache.winner(id(1), DataKind::DailyClose).unwrap().map(|w| w.1), Some("ONE.V".to_string()));
}

#[test]
fn a_past_span_is_undone_for_the_splits_after_it_too() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-24T04:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    // NVDA on 2024-05-17 (an expiry day), before its 10:1 split of 2024-06-10:
    // the one-day reply lists no split, the monthly chart since does
    let recorded = Arc::new(
        common::Recorded::new()
            .with(&format!("{YAHOO}NVDA?period1=1715990400&period2=1790294400&interval=1mo&events=split"), 200, "yahoo", "NVDA-splits-2024-05-18-2026-09-24.json")
            .with(&format!("{YAHOO}NVDA?period1=1715904000&period2=1715990400&interval=1d&events=div%7Csplit"), 200, "yahoo", "NVDA-2024-05-17-2024-05-17.json"),
    );
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let zone = eastern();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    let nvda = need(listing(1, InstrumentKind::Security, Currency::USD, "NVDA", Some("XNAS")), date(2024, 5, 17), date(2024, 5, 17));
    market::read_closes(&ctx, &[nvda]).unwrap();
    // as traded: Yahoo's 92.47899627685547 multiplied back by the later split
    assert_eq!(cache.closes().unwrap()[&id(1)][&date(2024, 5, 17)], Money::new(Dec::parse("924.7899627685547").unwrap(), Currency::USD));
}

#[test]
fn a_benchmark_reaches_back_when_the_oldest_day_moves_earlier() {
    // levels stored from 2022 by a read after they settled; then trades from 2015 arrive
    let state = CloseState { days: days(date(2022, 1, 3), date(2026, 9, 23)), reads: vec![read(date(2022, 1, 3), date(2026, 9, 23), OutcomeKind::Answered, "2026-09-23T21:00:00Z")] };
    let now = t("2026-09-24T12:00:00Z");
    assert_eq!(market::due_span(Market::Canada, date(2022, 1, 3), date(2026, 9, 24), &state, now, &eastern(), REST), None);
    assert_eq!(market::due_span(Market::Canada, date(2015, 6, 1), date(2026, 9, 24), &state, now, &eastern(), REST), Some((date(2015, 6, 1), date(2021, 12, 31))));
}
