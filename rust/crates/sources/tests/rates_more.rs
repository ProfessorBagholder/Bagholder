//! The Bank's rates and holidays through a whole run (`rates::read`) on recorded
//! replies: what each reply writes into the book, or that it writes nothing and
//! which outcome is recorded (`docs/plans/stage-3a-sources.md`, acceptance
//! criteria "Bank of Canada" and "Periodic reads").
//!
//! A recorded reply is served for the request the reader makes. Where a capture
//! was made for another span than the reader asks (the archives are asked whole,
//! era by era), the test says so beside it.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bagholder_book::facts::{RateConflict, RateSeries};
use bagholder_book::Book;
use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_sources::adapters::{boc, holidays, statcan};
use bagholder_sources::cache::{MarketCache, ReadRow};
use bagholder_sources::contract::DataKind;
use bagholder_sources::outcome::OutcomeKind;
use bagholder_sources::rates::{self, Held, Need};
use bagholder_sources::read::Ctx;

const VALET: &str = "https://www.bankofcanada.ca/valet";
const HOLIDAY_PAGE: &str = "https://www.bankofcanada.ca/press/upcoming-events/bank-of-canada-holiday-schedule/";
const WDS: &str = "https://www150.statcan.gc.ca/t1/wds/rest/getDataFromVectorByReferencePeriodRange";
const BOC: &str = "bank-of-canada";
const NOON: &str = "bank-of-canada-noon";
const STATCAN: &str = "statistics-canada";
const HOLIDAYS: &str = "bank-of-canada-holidays";

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn cur(s: &str) -> Currency {
    Currency::parse(s).unwrap()
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn eastern() -> TimeZone {
    TimeZone::get("America/Toronto").unwrap()
}

fn need(c: Currency, oldest: Date) -> Need {
    Need { currency: c, oldest }
}

fn observations(code: &str, span: Option<(Date, Date)>) -> String {
    match span {
        Some((from, to)) => format!("{VALET}/observations/{code}/json?start_date={from}&end_date={to}"),
        None => format!("{VALET}/observations/{code}/json"),
    }
}

fn group() -> String {
    format!("{VALET}/groups/FX_RATES_DAILY/json")
}

fn archive_url(vector: u32, from: Date, to: Date) -> String {
    format!("{WDS}?vectorIds=%22{vector}%22&startRefPeriod={from}&endReferencePeriod={to}")
}

/// A book and a market cache in a folder of their own.
struct Env {
    _dir: tempfile::TempDir,
    book: Book,
    cache: MarketCache,
    zone: TimeZone,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-01-01T00:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    Env { _dir: dir, book, cache, zone: eastern() }
}

impl Env {
    fn run(&self, recorded: &Arc<common::Recorded>, now: &str, needs: &[Need]) {
        let net = common::net(recorded, now);
        let ctx = Ctx { book: &self.book, cache: &self.cache, net: &net, now: t(now), bank: &self.zone };
        rates::read(&ctx, needs).unwrap();
    }

    /// The holiday page answered at `at`: it is not due again that month, so a
    /// run reads only the rates.
    fn holidays_read_at(&self, at: &str) {
        let d = t(at).to_zoned(eastern()).date();
        let subject = format!("holidays:{}", holidays::SOURCE);
        self.cache.store_read(&subject, DataKind::Rate, &ReadRow { source: holidays::source(), first: d, last: d, outcome: OutcomeKind::Answered, at: t(at) }).unwrap();
    }

    fn series(&self, s: &[RateSeries]) {
        self.book.store_rate_series(s, t("2026-01-01T00:00:00Z")).unwrap();
    }

    /// Everything the Bank's readers write into the book, without the times it
    /// was received.
    fn facts(&self) -> Facts {
        Facts {
            rates: self.book.rates().unwrap(),
            reads: self.book.rate_reads().unwrap().into_iter().map(|(c, r)| (c, r.into_iter().map(|(a, b, _)| (a, b)).collect())).collect(),
            series: self.book.rate_series().unwrap(),
            holidays: self.book.bank_holidays().unwrap(),
            conflicts: self.book.rate_conflicts().unwrap(),
        }
    }

    /// The kinds of every outcome recorded for `source`, oldest first.
    fn outcomes(&self, source: &'static str) -> Vec<OutcomeKind> {
        let mut v: Vec<OutcomeKind> = self.cache.outcomes(&SourceName::named(source)).unwrap().into_iter().map(|o| o.outcome).collect();
        v.reverse();
        v
    }

    fn details(&self, source: &'static str) -> Vec<String> {
        self.cache.outcomes(&SourceName::named(source)).unwrap().into_iter().map(|o| o.detail).collect()
    }

    /// How the newest read of a subject (`USD:bank-of-canada`) ended.
    fn last_read(&self, subject: &str) -> Option<OutcomeKind> {
        self.cache.reads(subject, DataKind::Rate).unwrap().first().map(|r| r.outcome)
    }
}

#[derive(Debug, PartialEq)]
struct Facts {
    rates: BTreeMap<Currency, BTreeMap<Date, Dec>>,
    reads: BTreeMap<Currency, Vec<(Date, Date)>>,
    series: Vec<RateSeries>,
    holidays: BTreeSet<Date>,
    conflicts: Vec<RateConflict>,
}

fn daily(c: Currency, first: Date, last: Date) -> RateSeries {
    RateSeries { currency: c, source: boc::daily_source(), first_day: first, last_day: last, ended: false }
}

fn asked(r: &Arc<common::Recorded>) -> Vec<String> {
    r.asked.lock().unwrap().clone()
}

// ---------------------------------------------------------------------------
// the noon table's description checked on every read
// ---------------------------------------------------------------------------

/// A book holding the dollar's daily series, so the only read due for a need
/// from 2016-12-29 is its noon archive.
fn usd_noon_only() -> (Env, [Need; 1]) {
    let e = env();
    e.series(&[daily(Currency::USD, date(2017, 1, 3), date(2026, 9, 23))]);
    e.holidays_read_at("2026-09-24T12:00:00Z");
    (e, [need(Currency::USD, date(2016, 12, 29))])
}

const NOW: &str = "2026-09-24T12:00:00Z";

#[test]
fn the_forward_rate_described_in_place_of_the_spot_rate_is_refused_and_writes_nothing() {
    let url = observations("IEXE0101", Some((date(2007, 5, 1), date(2017, 1, 2))));
    // IEXE0105's 90-day forward rates under the spot series' code, cut to the span asked
    let (e, needs) = usd_noon_only();
    let before = e.facts();
    let r = Arc::new(common::Recorded::new().with(&url, 200, NOON, "wrong-meaning-IEXE0101-forward-rate.json"));
    e.run(&r, NOW, &needs);
    assert_eq!(asked(&r), vec![url.clone()]);
    assert_eq!(e.facts(), before);
    assert!(e.book.rates().unwrap().is_empty());
    assert!(e.book.rate_reads().unwrap().is_empty());
    assert!(!e.book.rate_series().unwrap().iter().any(|s| s.source.as_str() == boc::NOON_SOURCE));
    assert_eq!(e.outcomes(boc::NOON_SOURCE), vec![OutcomeKind::Meaning]);
    assert!(e.details(boc::NOON_SOURCE)[0].contains("\"U.S. dollar noon, 90-day\""), "{:?}", e.details(boc::NOON_SOURCE));
    assert_eq!(e.last_read("USD:bank-of-canada-noon"), Some(OutcomeKind::Meaning));

    // the forward series' own reply, served where the spot series is asked: another series
    let (e, needs) = usd_noon_only();
    let r = Arc::new(common::Recorded::new().with(&url, 200, NOON, "IEXE0105-2016-12-28-2017-01-05.json"));
    e.run(&r, NOW, &needs);
    assert_eq!(e.facts(), before);
    assert_eq!(e.outcomes(boc::NOON_SOURCE), vec![OutcomeKind::Meaning]);
    assert!(e.details(boc::NOON_SOURCE)[0].contains("names the series IEXE0105, not IEXE0101"), "{:?}", e.details(boc::NOON_SOURCE));
    assert_eq!(e.last_read("USD:bank-of-canada-noon"), Some(OutcomeKind::Meaning));
}

#[test]
fn a_noon_series_answered_as_the_table_describes_it_is_stored_with_its_span() {
    // GHC's old cedi: its whole noon series is 2007-05-01 to 2007-06-29, as captured
    let e = env();
    e.holidays_read_at(NOW);
    let r = Arc::new(
        common::Recorded::new()
            .with(&group(), 200, BOC, "group-FX_RATES_DAILY.json")
            .with(&observations("IEXE4701", Some((date(2007, 5, 1), date(2007, 6, 29)))), 200, NOON, "IEXE4701-2007-05-01-2007-06-29.json"),
    );
    e.run(&r, NOW, &[need(cur("GHC"), date(2007, 5, 15))]);
    let f = e.facts();
    let ghc = &f.rates[&cur("GHC")];
    assert_eq!(ghc.len(), 43);
    assert_eq!(ghc.first_key_value(), Some((&date(2007, 5, 1), &dec("0.000119"))));
    assert_eq!(ghc.get(&date(2007, 5, 11)), Some(&dec("0.000120")));
    assert_eq!(ghc.last_key_value(), Some((&date(2007, 6, 29), &dec("0.000115"))));
    assert_eq!(f.reads[&cur("GHC")], vec![(date(2007, 5, 1), date(2007, 6, 29))]);
    assert_eq!(f.series, vec![RateSeries { currency: cur("GHC"), source: boc::noon_source(), first_day: date(2007, 5, 1), last_day: date(2007, 6, 29), ended: true }]);
    assert_eq!(e.outcomes(boc::NOON_SOURCE), vec![OutcomeKind::Answered]);
}

#[test]
fn a_noon_reply_missing_its_description_is_a_mismatch_writing_nothing() {
    let e = env();
    e.holidays_read_at(NOW);
    let r = Arc::new(
        common::Recorded::new()
            .with(&group(), 200, BOC, "group-FX_RATES_DAILY.json")
            .with(&observations("IEXE4701", Some((date(2007, 5, 1), date(2007, 6, 29)))), 200, NOON, "wrong-shape-IEXE4701-no-description.json"),
    );
    e.run(&r, NOW, &[need(cur("GHC"), date(2007, 5, 15))]);
    let f = e.facts();
    assert!(f.rates.is_empty() && f.reads.is_empty() && f.series.is_empty(), "{f:?}");
    assert_eq!(e.outcomes(boc::NOON_SOURCE), vec![OutcomeKind::Mismatch]);
    assert!(e.details(boc::NOON_SOURCE)[0].contains("seriesDetail.IEXE4701.description"), "{:?}", e.details(boc::NOON_SOURCE));
    assert_eq!(e.last_read("GHC:bank-of-canada-noon"), Some(OutcomeKind::Mismatch));
}

#[test]
fn the_local_pesos_days_in_the_plain_pesos_era_are_refused_and_the_currency_writes_nothing() {
    // IEXE2702 goes on past 2010-01-01 beside IEXE2703; the table stands it only to
    // 2010-01-01, so a reply stating 2010-01-04 for it is outside the span asked,
    // and the peso's second series is not asked
    let e = env();
    e.holidays_read_at(NOW);
    let plain = observations("IEXE2703", Some((date(2010, 1, 4), date(2017, 4, 28))));
    let r = Arc::new(
        common::Recorded::new()
            .with(&group(), 200, BOC, "group-FX_RATES_DAILY.json")
            .with(&observations("IEXE2702", Some((date(2007, 5, 1), date(2010, 1, 1)))), 200, NOON, "IEXE2702-2010-01-01-2010-01-08.json")
            .with(&plain, 200, NOON, "IEXE2703-2010-01-01-2010-01-08.json"),
    );
    e.run(&r, NOW, &[need(cur("ARS"), date(2008, 1, 2))]);
    assert!(!asked(&r).contains(&plain));
    let f = e.facts();
    assert!(f.rates.is_empty() && f.reads.is_empty() && f.series.is_empty(), "{f:?}");
    assert_eq!(e.outcomes(boc::NOON_SOURCE), vec![OutcomeKind::Meaning]);
    assert!(e.details(boc::NOON_SOURCE)[0].contains("IEXE2702 answered 2010-01-04, outside the 2007-05-01 to 2010-01-01 asked"), "{:?}", e.details(boc::NOON_SOURCE));
}

// ---------------------------------------------------------------------------
// a span clamped to the series' days, and the eras in either order
// ---------------------------------------------------------------------------

/// The dollar's whole daily series as it stood on the evening of 2017-01-05: its
/// first three days. The capture was asked from 2016-12-28; the series answered
/// from its first day, 2017-01-03, as the whole series then would.
fn usd_daily_whole(r: common::Recorded) -> common::Recorded {
    r.with(&observations("FXUSDCAD", None), 200, BOC, "FXUSDCAD-2016-12-28-2017-01-05.json")
}

const EVENING: &str = "2017-01-05T23:00:00Z";

#[test]
fn a_need_from_2010_records_the_daily_span_from_the_series_first_day() {
    let e = env();
    e.holidays_read_at(EVENING);
    let noon = observations("IEXE0101", Some((date(2007, 5, 1), date(2017, 1, 2))));
    let r = Arc::new(usd_daily_whole(common::Recorded::new().with(&group(), 200, BOC, "group-FX_RATES_DAILY.json")).with(&noon, 200, NOON, "IEXE0101-2016-12-28-2017-01-05.json"));
    e.run(&r, EVENING, &[need(Currency::USD, date(2010, 1, 4))]);
    let f = e.facts();
    assert_eq!(f.reads[&Currency::USD], vec![(date(2017, 1, 3), date(2017, 1, 5))]);
    assert_eq!(f.rates[&Currency::USD], BTreeMap::from([(date(2017, 1, 3), dec("1.3435")), (date(2017, 1, 4), dec("1.3315")), (date(2017, 1, 5), dec("1.3244"))]));
    assert_eq!(f.series, vec![daily(Currency::USD, date(2017, 1, 3), date(2017, 1, 5))]);
    // the days before it are the noon archive's to state, asked only to 2017-01-02
    assert_eq!(asked(&r), vec![group(), observations("FXUSDCAD", None), noon]);
}

/// The dollar needed from 2007-04-25: its daily series, its noon archive and
/// Statistics Canada's. Each archive's capture states days of the next era
/// (IEXE0101 to 2017-01-05, v121716 to 2007-05-04), so each is refused whole.
fn eras(r: common::Recorded) -> common::Recorded {
    r.with(&observations("IEXE0101", Some((date(2007, 5, 1), date(2017, 1, 2)))), 200, NOON, "IEXE0101-2016-12-28-2017-01-05.json").with(&archive_url(121716, date(1950, 10, 2), date(2007, 4, 30)), 200, STATCAN, "v121716-2007-04-25-2007-05-04.json")
}

#[test]
fn each_era_stops_before_the_next_whichever_read_runs_first() {
    let needs = [need(Currency::USD, date(2007, 4, 25))];
    // the daily series first, then the archives, in one run
    let a = env();
    a.holidays_read_at(EVENING);
    let ra = Arc::new(eras(usd_daily_whole(common::Recorded::new().with(&group(), 200, BOC, "group-FX_RATES_DAILY.json"))));
    a.run(&ra, EVENING, &needs);
    assert_eq!(asked(&ra).len(), 4);

    // the archives first, while the group fails, then the daily series a day later
    let b = env();
    b.holidays_read_at(EVENING);
    let rb = Arc::new(eras(common::Recorded::new().with(&group(), 500, BOC, "group-FX_RATES_DAILY.json")));
    b.run(&rb, EVENING, &needs);
    assert_eq!(asked(&rb).len(), 3);
    assert!(b.facts().rates.is_empty());
    let rb2 = Arc::new(eras(usd_daily_whole(common::Recorded::new().with(&group(), 200, BOC, "group-FX_RATES_DAILY.json"))));
    b.run(&rb2, "2017-01-06T23:00:00Z", &needs);
    assert_eq!(asked(&rb2).len(), 4);

    let (fa, fb) = (a.facts(), b.facts());
    assert_eq!(fa, fb);
    assert!(fa.conflicts.is_empty());
    // the daily era's first day is the daily series' rate, never the noon archive's 1.3438,
    // and the noon era's first day never Statistics Canada's 1.1089
    assert_eq!(fa.rates[&Currency::USD].keys().copied().collect::<Vec<_>>(), vec![date(2017, 1, 3), date(2017, 1, 4), date(2017, 1, 5)]);
    assert_eq!(fa.rates[&Currency::USD][&date(2017, 1, 3)], dec("1.3435"));
    assert_eq!(fa.reads[&Currency::USD], vec![(date(2017, 1, 3), date(2017, 1, 5))]);
    assert_eq!(fa.series, vec![daily(Currency::USD, date(2017, 1, 3), date(2017, 1, 5))]);
    for e in [&a, &b] {
        assert!(e.details(boc::NOON_SOURCE).iter().all(|d| d.contains("IEXE0101 answered 2017-01-03, outside the 2007-05-01 to 2017-01-02 asked")), "{:?}", e.details(boc::NOON_SOURCE));
        assert!(e.details(statcan::SOURCE).iter().all(|d| d.contains("vector 121716 answered 2007-05-01, outside the 1950-10-02 to 2007-04-30 asked")), "{:?}", e.details(statcan::SOURCE));
        assert_eq!(e.last_read("USD:bank-of-canada-noon"), Some(OutcomeKind::Meaning));
        assert_eq!(e.last_read("USD:statistics-canada"), Some(OutcomeKind::Meaning));
    }
    assert_eq!(a.outcomes(statcan::SOURCE), vec![OutcomeKind::Meaning]);
    assert_eq!(b.outcomes(statcan::SOURCE), vec![OutcomeKind::Meaning, OutcomeKind::Meaning]);
}

// ---------------------------------------------------------------------------
// Statistics Canada's archive through a run
// ---------------------------------------------------------------------------

/// A book holding `c`'s daily and noon series, so the only read due for a need
/// from 1950-10-03 is Statistics Canada's archive.
fn archive_only(c: Currency) -> Env {
    let e = env();
    e.series(&[daily(c, date(2017, 1, 3), date(2026, 9, 23)), RateSeries { currency: c, source: boc::noon_source(), first_day: date(2007, 5, 1), last_day: date(2017, 1, 2), ended: true }]);
    e.holidays_read_at(NOW);
    e
}

#[test]
fn the_archive_stores_business_days_to_the_eras_end() {
    // the pound's capture, 1950-10-02 to 1950-10-10, served for the era the reader asks
    let e = archive_only(cur("GBP"));
    let before = e.book.rate_series().unwrap();
    let r = Arc::new(common::Recorded::new().with(&archive_url(121720, date(1950, 10, 2), date(2007, 4, 30)), 200, STATCAN, "v121720-1950-10-02-1950-10-10.json"));
    e.run(&r, NOW, &[need(cur("GBP"), date(1950, 10, 3))]);
    let f = e.facts();
    // the weekend's 0 and the holiday's null are no rates
    assert_eq!(
        f.rates[&cur("GBP")],
        BTreeMap::from([
            (date(1950, 10, 2), dec("2.96799999")),
            (date(1950, 10, 3), dec("2.933")),
            (date(1950, 10, 4), dec("2.94")),
            (date(1950, 10, 5), dec("2.961")),
            (date(1950, 10, 6), dec("2.96450001")),
            (date(1950, 10, 9), dec("2.961")),
        ])
    );
    assert_eq!(f.reads[&cur("GBP")], vec![(date(1950, 10, 2), statcan::ERA_END)]);
    let mut series = before;
    series.push(RateSeries { currency: cur("GBP"), source: statcan::source(), first_day: date(1950, 10, 2), last_day: date(2007, 4, 30), ended: true });
    series.sort_by_key(|s| s.first_day);
    assert_eq!(f.series, series);
    assert_eq!(e.outcomes(statcan::SOURCE), vec![OutcomeKind::Answered]);
}

#[test]
fn a_wrong_archive_reply_writes_nothing_through_a_run() {
    for (name, kind, says) in [
        ("wrong-shape-v121716-value-as-text.json", OutcomeKind::Mismatch, "[0].object.vectorDataPoint[0].value"),
        ("wrong-meaning-v121716-zero-on-a-weekday.json", OutcomeKind::Meaning, "for 2007-04-25"),
        ("wrong-meaning-v121716-another-vector.json", OutcomeKind::Meaning, "vector 121717"),
        ("v121716-2007-04-25-2007-05-04.json", OutcomeKind::Meaning, "answered 2007-05-01, outside"),
    ] {
        let e = archive_only(Currency::USD);
        let before = e.facts();
        let r = Arc::new(common::Recorded::new().with(&archive_url(121716, date(1950, 10, 2), date(2007, 4, 30)), 200, STATCAN, name));
        e.run(&r, NOW, &[need(Currency::USD, date(1950, 10, 3))]);
        assert_eq!(asked(&r).len(), 1, "{name}");
        assert_eq!(e.facts(), before, "{name}");
        assert_eq!(e.outcomes(statcan::SOURCE), vec![kind], "{name}");
        assert!(e.details(statcan::SOURCE)[0].contains(says), "{name}: {:?}", e.details(statcan::SOURCE));
        assert_eq!(e.last_read("USD:statistics-canada"), Some(kind), "{name}");
    }
}

// ---------------------------------------------------------------------------
// a wrong daily reply writes nothing
// ---------------------------------------------------------------------------

/// A book whose dollar series was read to `last`; at 17:00 Eastern on 2026-09-08
/// the series is read forward from the next business day.
fn usd_forward(last: Date) -> Env {
    let e = env();
    e.series(&[daily(Currency::USD, date(2017, 1, 3), last)]);
    e.holidays_read_at(FORWARD_AT);
    e
}

const FORWARD_AT: &str = "2026-09-08T21:00:00Z";

#[test]
fn a_forward_read_stores_its_days_and_its_span() {
    let e = usd_forward(date(2026, 8, 27));
    let r = Arc::new(common::Recorded::new().with(&observations("FXUSDCAD", Some((date(2026, 8, 28), date(2026, 9, 8)))), 200, BOC, "FXUSDCAD-2026-08-28-2026-09-08.json"));
    e.run(&r, FORWARD_AT, &[need(Currency::USD, date(2020, 1, 2))]);
    let f = e.facts();
    let usd = &f.rates[&Currency::USD];
    assert_eq!(usd.len(), 7);
    assert_eq!(usd.first_key_value(), Some((&date(2026, 8, 28), &dec("1.3888"))));
    assert_eq!(usd.last_key_value(), Some((&date(2026, 9, 8), &dec("1.3784"))));
    assert!(!usd.contains_key(&date(2026, 9, 7)));
    assert_eq!(f.reads[&Currency::USD], vec![(date(2026, 8, 28), date(2026, 9, 8))]);
    // a forward read keeps the series' first day
    assert_eq!(f.series, vec![daily(Currency::USD, date(2017, 1, 3), date(2026, 9, 8))]);
    assert_eq!(e.outcomes(boc::DAILY), vec![OutcomeKind::Answered]);
}

#[test]
fn a_repeated_day_another_series_a_non_decimal_value_and_a_day_outside_the_span_each_write_nothing() {
    let span = Some((date(2026, 8, 28), date(2026, 9, 8)));
    for (name, kind, says) in [
        ("wrong-meaning-FXUSDCAD-day-repeated.json", OutcomeKind::Meaning, "a day repeated or out of order"),
        ("wrong-meaning-FXUSDCAD-another-series.json", OutcomeKind::Meaning, "names the series FXEURCAD, not FXUSDCAD"),
        ("wrong-meaning-FXUSDCAD-negative-rate.json", OutcomeKind::Meaning, "-1.3888"),
        ("wrong-shape-FXUSDCAD-rate-not-text.json", OutcomeKind::Mismatch, "observations[0].FXUSDCAD.v"),
        ("wrong-shape-FXUSDCAD-rate-not-a-decimal.json", OutcomeKind::Mismatch, "observations[0].FXUSDCAD.v"),
    ] {
        let e = usd_forward(date(2026, 8, 27));
        let before = e.facts();
        let r = Arc::new(common::Recorded::new().with(&observations("FXUSDCAD", span), 200, BOC, name));
        e.run(&r, FORWARD_AT, &[need(Currency::USD, date(2020, 1, 2))]);
        assert_eq!(asked(&r).len(), 1, "{name}");
        assert_eq!(e.facts(), before, "{name}");
        assert_eq!(e.outcomes(boc::DAILY), vec![kind], "{name}");
        assert!(e.details(boc::DAILY)[0].contains(says), "{name}: {:?}", e.details(boc::DAILY));
        assert_eq!(e.last_read("USD:bank-of-canada"), Some(kind), "{name}");
    }
    // the real reply, served for a read asked from 2026-09-01: 2026-08-28 is outside it
    let e = usd_forward(date(2026, 8, 31));
    let before = e.facts();
    let r = Arc::new(common::Recorded::new().with(&observations("FXUSDCAD", Some((date(2026, 9, 1), date(2026, 9, 8)))), 200, BOC, "FXUSDCAD-2026-08-28-2026-09-08.json"));
    e.run(&r, FORWARD_AT, &[need(Currency::USD, date(2020, 1, 2))]);
    assert_eq!(e.facts(), before);
    assert_eq!(e.outcomes(boc::DAILY), vec![OutcomeKind::Meaning]);
    assert!(e.details(boc::DAILY)[0].contains("FXUSDCAD answered 2026-08-28, outside the 2026-09-01 to 2026-09-08 asked"), "{:?}", e.details(boc::DAILY));
    assert_eq!(e.last_read("USD:bank-of-canada"), Some(OutcomeKind::Meaning));
}

// ---------------------------------------------------------------------------
// the holiday page
// ---------------------------------------------------------------------------

#[test]
fn the_holiday_pages_closures_are_stored_and_a_wrong_page_writes_nothing() {
    let e = env();
    let r = Arc::new(common::Recorded::new().with(HOLIDAY_PAGE, 200, HOLIDAYS, "page-2026-09-24.html"));
    e.run(&r, "2026-09-24T12:00:00Z", &[]);
    assert_eq!(e.book.bank_holidays().unwrap(), BTreeSet::from([date(2026, 9, 30), date(2026, 10, 12), date(2026, 11, 11), date(2026, 12, 25), date(2026, 12, 28)]));
    assert_eq!(e.outcomes(holidays::SOURCE), vec![OutcomeKind::Answered]);

    for (name, kind, says) in [
        ("wrong-shape-no-date.html", OutcomeKind::Mismatch, "article 0's date \"\" is not a day"),
        ("wrong-shape-no-closures.html", OutcomeKind::Mismatch, "lists no closures"),
        // last year's page, read in 2026: no closure still to come
        ("wrong-meaning-last-years-dates.html", OutcomeKind::Meaning, "no closure on or after 2026-09-24 (its last is 2025-12-28): it is not current"),
    ] {
        let e = env();
        let r = Arc::new(common::Recorded::new().with(HOLIDAY_PAGE, 200, HOLIDAYS, name));
        e.run(&r, "2026-09-24T12:00:00Z", &[]);
        assert!(e.book.bank_holidays().unwrap().is_empty(), "{name}");
        assert_eq!(e.outcomes(holidays::SOURCE), vec![kind], "{name}");
        assert!(e.details(holidays::SOURCE)[0].contains(says), "{name}: {:?}", e.details(holidays::SOURCE));
        assert_eq!(e.last_read("holidays:bank-of-canada-holidays"), Some(kind), "{name}");
    }
}

#[test]
fn a_failed_or_refused_holiday_read_waits_out_its_rest() {
    for (status, name) in [(500, "page-2026-09-24.html"), (429, "page-2026-09-24.html"), (200, "wrong-meaning-last-years-dates.html")] {
        let e = env();
        let r = Arc::new(common::Recorded::new().with(HOLIDAY_PAGE, status, HOLIDAYS, name));
        e.run(&r, "2026-10-01T12:00:00Z", &[]);
        assert_eq!(asked(&r).len(), 1, "{status} {name}");
        assert!(e.book.bank_holidays().unwrap().is_empty());
        e.run(&r, "2026-10-01T12:00:30Z", &[]);
        assert_eq!(asked(&r).len(), 1, "{status} {name}: asked again within its rest");
        e.run(&r, "2026-10-01T12:01:00Z", &[]);
        assert_eq!(asked(&r).len(), 2, "{status} {name}: not asked again once its rest is over");
    }
}

fn read_at(at: &str, holidays: &[Date]) -> Held {
    Held { holidays_read: Some(t(at)), holidays: holidays.iter().copied().collect(), ..Held::default() }
}

#[test]
fn the_holiday_page_is_not_read_again_the_month_it_was_read() {
    let z = eastern();
    // read on 2026-10-01, the first business day of October
    let held = read_at("2026-10-01T14:00:00Z", &[]);
    assert!(!rates::due_holidays(&held, t("2026-10-02T14:00:00Z"), &z));
    assert!(!rates::due_holidays(&held, t("2026-10-31T23:00:00Z"), &z));
    // 2026-11-01T03:00Z is still October 31st in Toronto
    assert!(!rates::due_holidays(&held, t("2026-11-01T03:00:00Z"), &z));
}

#[test]
fn the_holiday_page_is_due_from_the_first_day_of_each_month_and_not_again_that_month() {
    // the page names this year's closures ahead of them: reading it from the month's
    // first day, a weekend or a Bank holiday included, knows each closure before its day
    let z = eastern();
    let sept = read_at("2026-09-02T14:00:00Z", &[]);
    assert!(!rates::due_holidays(&sept, t("2026-09-30T14:00:00Z"), &z));
    assert!(rates::due_holidays(&sept, t("2026-10-01T14:00:00Z"), &z));
    // Sunday 2026-11-01, and the Bank's holiday 2027-01-01, in the Bank's zone
    let oct = read_at("2026-10-01T14:00:00Z", &[]);
    assert!(!rates::due_holidays(&oct, t("2026-11-01T03:59:00Z"), &z), "still October in Eastern time");
    assert!(rates::due_holidays(&oct, t("2026-11-01T15:00:00Z"), &z));
    let dec_read = read_at("2026-12-01T15:00:00Z", &[date(2027, 1, 1)]);
    assert!(rates::due_holidays(&dec_read, t("2027-01-01T15:00:00Z"), &z));
    // and not again that month once read
    let jan = read_at("2027-01-01T15:00:00Z", &[date(2027, 1, 1)]);
    assert!(!rates::due_holidays(&jan, t("2027-01-29T15:00:00Z"), &z));
}
