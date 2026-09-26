//! When the Bank's rates are read (on a clock handed in), and what a run stores
//! in the book from recorded replies.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use bagholder_book::facts::RateSeries;
use bagholder_book::Book;
use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, SourceName};
use bagholder_sources::adapters::{boc, holidays, statcan};
use bagholder_sources::cache::MarketCache;
use bagholder_sources::health::{self, State};
use bagholder_sources::rates::{self, Due, Held, Need};
use bagholder_sources::read::Ctx;

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn cur(s: &str) -> Currency {
    Currency::parse(s).unwrap()
}

fn eastern() -> TimeZone {
    TimeZone::get("America/Toronto").unwrap()
}

fn daily(c: Currency, first: Date, last: Date, ended: bool) -> RateSeries {
    RateSeries { currency: c, source: boc::daily_source(), first_day: first, last_day: last, ended }
}

fn need(c: Currency, oldest: Date) -> Need {
    Need { currency: c, oldest }
}

#[test]
fn the_group_is_read_for_a_currency_no_series_holds_at_most_once_a_day() {
    let usd = [need(Currency::USD, date(2020, 1, 2))];
    let fresh = Held::default();
    assert_eq!(rates::due_daily(&usd, &fresh, t("2026-09-24T12:00:00Z"), &eastern()), vec![Due::DailyList]);
    let listed = Held { list_read: Some(t("2026-09-24T00:00:00Z")), ..Held::default() };
    assert!(rates::due_daily(&usd, &listed, t("2026-09-24T12:00:00Z"), &eastern()).is_empty());
    assert_eq!(rates::due_daily(&usd, &listed, t("2026-09-25T00:00:00Z"), &eastern()), vec![Due::DailyList]);
    // a currency the Bank lists and the book does not hold is read whole
    let list: BTreeSet<Currency> = [Currency::USD].into();
    assert_eq!(rates::due_daily_whole(&usd, &fresh, &list), vec![Due::DailyWhole(Currency::USD)]);
    assert!(rates::due_daily_whole(&[need(cur("XAU"), date(2020, 1, 2))], &fresh, &list).is_empty());
}

fn holding_usd(last: Date, holidays: &[Date]) -> Held {
    Held {
        series: vec![daily(Currency::USD, date(2017, 1, 3), last, false)],
        // each read received the evening after its last day
        reads: [(Currency::USD, vec![(date(2017, 1, 3), last, last.at(23, 0, 0, 0).to_zoned(eastern()).unwrap().timestamp())])].into(),
        holidays: holidays.iter().copied().collect(),
        ..Held::default()
    }
}

#[test]
fn a_daily_series_is_read_forward_once_the_next_business_days_rate_is_due() {
    let usd = [need(Currency::USD, date(2020, 1, 2))];
    let held = holding_usd(date(2026, 9, 22), &[]);
    // 16:00 Eastern on the 23rd: not published yet
    assert!(rates::due_daily(&usd, &held, t("2026-09-23T20:00:00Z"), &eastern()).is_empty());
    // 16:31 Eastern: due, from the day after the last read to today
    assert_eq!(rates::due_daily(&usd, &held, t("2026-09-23T20:31:00Z"), &eastern()), vec![Due::DailyForward(Currency::USD, date(2026, 9, 23), date(2026, 9, 23))]);
    // a Friday read: nothing is due over the weekend, Monday after 16:30 is
    let friday = holding_usd(date(2026, 9, 25), &[]);
    assert!(rates::due_daily(&usd, &friday, t("2026-09-27T23:00:00Z"), &eastern()).is_empty());
    assert!(rates::due_daily(&usd, &friday, t("2026-09-28T19:00:00Z"), &eastern()).is_empty());
    assert_eq!(rates::due_daily(&usd, &friday, t("2026-09-28T20:31:00Z"), &eastern()), vec![Due::DailyForward(Currency::USD, date(2026, 9, 26), date(2026, 9, 28))]);
    // the Bank's holiday is no business day
    let holiday = holding_usd(date(2026, 9, 29), &[date(2026, 9, 30)]);
    assert!(rates::due_daily(&usd, &holiday, t("2026-09-30T23:00:00Z"), &eastern()).is_empty());
    // a series that has ended is never read again
    let ended = Held { series: vec![daily(cur("VND"), date(2017, 1, 3), date(2019, 12, 31), true)], ..Held::default() };
    assert!(rates::due_daily(&[need(cur("VND"), date(2018, 1, 2))], &ended, t("2026-09-30T23:00:00Z"), &eastern()).is_empty());
}

#[test]
fn the_archives_are_read_once_and_only_for_days_before_the_daily_series() {
    let held = holding_usd(date(2026, 9, 22), &[]);
    assert!(rates::due_archives(&[need(Currency::USD, date(2018, 1, 2))], &held).is_empty());
    assert_eq!(rates::due_archives(&[need(Currency::USD, date(2016, 12, 29))], &held), vec![Due::Noon(Currency::USD)]);
    assert_eq!(rates::due_archives(&[need(Currency::USD, date(2005, 6, 1))], &held), vec![Due::Noon(Currency::USD), Due::Archive(Currency::USD)]);
    let mut both = held.clone();
    both.series.push(RateSeries { currency: Currency::USD, source: boc::noon_source(), first_day: date(2007, 5, 1), last_day: date(2017, 1, 2), ended: true });
    both.series.push(RateSeries { currency: Currency::USD, source: statcan::source(), first_day: date(1950, 10, 2), last_day: statcan::ERA_END, ended: true });
    assert!(rates::due_archives(&[need(Currency::USD, date(2005, 6, 1))], &both).is_empty());
    // a currency with no daily series: its noon archive whenever it is needed
    assert_eq!(rates::due_archives(&[need(cur("GHC"), date(2007, 5, 15))], &Held::default()), vec![Due::Noon(cur("GHC"))]);
    // the riyal has no archive
    assert!(rates::due_archives(&[need(cur("SAR"), date(2005, 6, 1))], &Held::default()).is_empty());
}

#[test]
fn the_holiday_page_is_read_once_a_month_in_the_banks_zone() {
    assert!(rates::due_holidays(&Held::default(), t("2026-09-24T12:00:00Z"), &eastern()));
    let read = Held { holidays_read: Some(t("2026-09-02T12:00:00Z")), ..Held::default() };
    assert!(!rates::due_holidays(&read, t("2026-09-30T23:00:00Z"), &eastern()));
    // 03:00 UTC on October 1st is still September 30th in Toronto
    assert!(!rates::due_holidays(&read, t("2026-10-01T03:00:00Z"), &eastern()));
    assert!(rates::due_holidays(&read, t("2026-10-01T05:00:00Z"), &eastern()));
}

const VALET: &str = "https://www.bankofcanada.ca/valet";

#[test]
fn a_run_stores_each_series_its_days_and_the_holidays_and_asks_again_only_when_due() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-24T04:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(
        common::Recorded::new()
            .with(&format!("{VALET}/groups/FX_RATES_DAILY/json"), 200, "bank-of-canada", "group-FX_RATES_DAILY.json")
            .with(&format!("{VALET}/observations/FXPLNCAD/json"), 200, "bank-of-canada", "FXPLNCAD-whole.json")
            .with(&format!("{VALET}/observations/IEXE4701/json?start_date=2007-05-01&end_date=2007-06-29"), 200, "bank-of-canada-noon", "IEXE4701-2007-05-01-2007-06-29.json")
            .with(&format!("{VALET}/observations/FXPLNCAD/json?start_date=2026-09-24&end_date=2026-09-24"), 200, "bank-of-canada", "FXPLNCAD-2026-09-24-2026-09-24.json")
            .with("https://www.bankofcanada.ca/press/upcoming-events/bank-of-canada-holiday-schedule/", 200, "bank-of-canada-holidays", "page-2026-09-24.html"),
    );
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let zone = eastern();
    let needs = [need(cur("PLN"), date(2026, 6, 1)), need(cur("GHC"), date(2007, 5, 15)), need(Currency::CAD, date(2007, 1, 2))];
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    rates::read(&ctx, &needs).unwrap();

    let series = book.rate_series().unwrap();
    let pln = series.iter().find(|s| s.currency == cur("PLN")).unwrap();
    assert_eq!((pln.first_day, pln.last_day, pln.ended, pln.source.as_str()), (date(2026, 5, 1), date(2026, 9, 23), false, boc::DAILY));
    let ghc = series.iter().find(|s| s.currency == cur("GHC")).unwrap();
    assert_eq!((ghc.first_day, ghc.last_day, ghc.ended, ghc.source.as_str()), (date(2007, 5, 1), date(2007, 6, 29), true, boc::NOON_SOURCE));
    let stored = book.rates().unwrap();
    assert_eq!(stored[&cur("PLN")].len(), 100);
    assert!(!stored[&cur("GHC")].is_empty());
    // the daily read covers to the last day it observed; the noon read its own span
    let reads = book.rate_reads().unwrap();
    assert_eq!(reads[&cur("PLN")].iter().map(|r| (r.0, r.1)).collect::<Vec<_>>(), vec![(date(2026, 5, 1), date(2026, 9, 23))]);
    assert_eq!(reads[&cur("GHC")].iter().map(|r| (r.0, r.1)).collect::<Vec<_>>(), vec![(date(2007, 5, 1), date(2007, 6, 29))]);
    assert_eq!(book.bank_holidays().unwrap().len(), 5);
    // every request's outcome recorded, and each source working
    for s in [boc::daily_source(), boc::noon_source(), holidays::source()] {
        assert_eq!(health::state(&cache.outcomes(&s).unwrap(), at), State::Working, "{s}");
    }
    assert_eq!(recorded.asked.lock().unwrap().len(), 4);

    // the same moment again: nothing is due
    rates::read(&ctx, &needs).unwrap();
    assert_eq!(recorded.asked.lock().unwrap().len(), 4);

    // after 16:30 Eastern on the 24th, the zloty is read forward; the Bank's
    // service has not posted the day yet, so the read settles nothing: the day
    // is not one the Bank did not publish, and it stays due
    let later = t("2026-09-24T21:00:00Z");
    let ctx = Ctx { now: later, ..ctx };
    rates::read(&ctx, &needs).unwrap();
    let asked = recorded.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 5);
    assert!(asked[4].ends_with("FXPLNCAD/json?start_date=2026-09-24&end_date=2026-09-24"));
    assert_eq!(book.rate_reads().unwrap()[&cur("PLN")].len(), 1);
    assert_eq!(cache.outcomes(&SourceName::named(boc::DAILY)).unwrap().len(), 3);
    let ctx = Ctx { now: t("2026-09-24T21:30:00Z"), ..ctx };
    rates::read(&ctx, &needs).unwrap();
    assert_eq!(recorded.asked.lock().unwrap().len(), 6);
}

#[test]
fn a_failed_read_of_the_bank_waits_out_its_rest() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-24T04:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    // the group of daily series answers 500
    let recorded = Arc::new(common::Recorded::new().with(&format!("{VALET}/groups/FX_RATES_DAILY/json"), 500, "bank-of-canada", "group-FX_RATES_DAILY.json").with("https://www.bankofcanada.ca/press/upcoming-events/bank-of-canada-holiday-schedule/", 200, "bank-of-canada-holidays", "page-2026-09-24.html")
        // the zloty's noon archive, asked while no daily series is known, also fails
        .with(&format!("{VALET}/observations/IEXE2401/json?start_date=2007-05-01&end_date=2017-04-28"), 500, "bank-of-canada", "group-FX_RATES_DAILY.json"));
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let zone = eastern();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    let needs = [need(cur("PLN"), date(2026, 6, 1))];
    rates::read(&ctx, &needs).unwrap();
    let asked = |r: &Arc<common::Recorded>, what: &str| r.asked.lock().unwrap().iter().filter(|u| u.contains(what)).count();
    assert_eq!((asked(&recorded, "FX_RATES_DAILY"), asked(&recorded, "IEXE2401")), (1, 1));
    rates::read(&Ctx { now: t("2026-09-24T04:00:30Z"), ..ctx }, &needs).unwrap();
    assert_eq!((asked(&recorded, "FX_RATES_DAILY"), asked(&recorded, "IEXE2401")), (1, 1));
    rates::read(&Ctx { now: t("2026-09-24T04:01:00Z"), ..ctx }, &needs).unwrap();
    assert_eq!((asked(&recorded, "FX_RATES_DAILY"), asked(&recorded, "IEXE2401")), (2, 2));
}
