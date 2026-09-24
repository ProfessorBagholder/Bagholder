//! The Bank of Canada's rates and holidays, read from recorded real replies:
//! exactly what each answer states, and each wrong-shaped or wrong-meaning copy
//! refused with nothing taken from it.

mod common;

use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::{Currency, Dec};
use bagholder_sources::adapters::{boc, holidays, statcan};
use bagholder_sources::outcome::{Outcome, OutcomeKind};

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn cur(s: &str) -> Currency {
    Currency::parse(s).unwrap()
}

const BOC: &str = "bank-of-canada";
const NOON: &str = "bank-of-canada-noon";
const STATCAN: &str = "statistics-canada";
const HOLIDAYS: &str = "bank-of-canada-holidays";

#[test]
fn the_recorded_shapes_are_the_answers_union() {
    common::shape_is_the_answers_union("bank-of-canada-group.paths", BOC, "group-", boc::GROUP_KEYED);
    common::shape_is_the_answers_union("bank-of-canada-observations.paths", BOC, "FX", boc::OBSERVATIONS_KEYED);
    common::shape_is_the_answers_union("bank-of-canada-observations.paths", NOON, "IEXE", boc::OBSERVATIONS_KEYED);
    common::shape_is_the_answers_union("statistics-canada.paths", STATCAN, "v", &[]);
}

#[test]
fn the_daily_group_lists_every_currency_the_bank_publishes_daily() {
    let Outcome::Answered(list) = boc::parse_daily_list(&common::json(BOC, "group-FX_RATES_DAILY.json")) else { panic!() };
    assert_eq!(list.len(), 27);
    assert!(list.contains(&boc::DailySeries { currency: Currency::USD, code: "FXUSDCAD".into() }));
    assert!(list.iter().any(|s| s.currency == cur("PLN")));
    assert!(matches!(boc::parse_daily_list(&common::json(BOC, "wrong-shape-group-no-label.json")), Outcome::Mismatch(m) if m.path == "groupDetails.groupSeries.FXUSDCAD.label"));
    assert!(matches!(boc::parse_daily_list(&common::json(BOC, "wrong-meaning-group-another-group.json")), Outcome::Meaning(w) if w.contains("FX_RATES_MONTHLY")));
}

#[test]
fn a_span_of_daily_rates_reads_exactly_as_written() {
    let span = (date(2026, 8, 28), date(2026, 9, 8));
    let Outcome::Answered(o) = boc::parse_observations(&common::json(BOC, "FXUSDCAD-2026-08-28-2026-09-08.json"), "FXUSDCAD", Some(span)) else { panic!() };
    assert_eq!(o.rates.first(), Some(&(date(2026, 8, 28), dec("1.3888"))));
    // Labour Day (2026-09-07) is not in the reply: the Bank did not publish
    assert!(o.rates.iter().all(|(d, _)| *d != date(2026, 9, 7)));
    assert!(o.rates.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(boc::daily_description(&o.description), Ok(false));
}

#[test]
fn a_historical_series_says_it_ended() {
    let span = (date(2019, 12, 20), date(2020, 1, 10));
    let Outcome::Answered(o) = boc::parse_observations(&common::json(BOC, "FXVNDCAD-2019-12-20-2020-01-10.json"), "FXVNDCAD", Some(span)) else { panic!() };
    assert_eq!(o.rates.last().map(|r| r.0), Some(date(2019, 12, 31)));
    assert_eq!(boc::daily_description(&o.description), Ok(true));
    // the zloty's whole series begins on 2026-05-01 and goes on
    let Outcome::Answered(pln) = boc::parse_observations(&common::json(BOC, "FXPLNCAD-whole.json"), "FXPLNCAD", None) else { panic!() };
    assert_eq!(pln.rates.first().map(|r| r.0), Some(date(2026, 5, 1)));
    assert_eq!(boc::daily_description(&pln.description), Ok(false));
    assert!(boc::daily_description("Monthly average exchange rate of the US dollar").is_err());
}

#[test]
fn a_wrong_daily_reply_writes_nothing() {
    let span = Some((date(2026, 8, 28), date(2026, 9, 8)));
    let read = |name: &str| boc::parse_observations(&common::json(BOC, name), "FXUSDCAD", span);
    assert!(matches!(read("wrong-shape-FXUSDCAD-rate-not-text.json"), Outcome::Mismatch(m) if m.path == "observations[0].FXUSDCAD.v"));
    assert!(matches!(read("wrong-meaning-FXUSDCAD-negative-rate.json"), Outcome::Meaning(w) if w.contains("-1.3888")));
    assert!(matches!(read("wrong-meaning-FXUSDCAD-day-repeated.json"), Outcome::Meaning(w) if w.contains("repeated")));
    assert!(matches!(read("wrong-meaning-FXUSDCAD-another-series.json"), Outcome::Meaning(w) if w.contains("FXEURCAD")));
    // a day outside the span asked
    let narrow = boc::parse_observations(&common::json(BOC, "FXUSDCAD-2026-08-28-2026-09-08.json"), "FXUSDCAD", Some((date(2026, 9, 1), date(2026, 9, 8))));
    assert!(matches!(narrow, Outcome::Meaning(w) if w.contains("outside")));
}

#[test]
fn a_series_the_bank_does_not_publish_is_not_carried() {
    let reply = bagholder_net::Reply { status: 404, url: "u".into(), headers: vec![], body: common::read(BOC, "FXXAUCAD-status-404.json").into_bytes(), received_at: "2026-09-24T00:00:00Z".parse().unwrap() };
    assert_eq!(bagholder_sources::ask::status(reply, &[404]).kind(), OutcomeKind::NotCarried);
}

#[test]
fn the_noon_table_is_the_banks_own_and_each_era_stops_before_the_next() {
    // the table names each series as the Bank describes it
    let span = (date(2016, 12, 28), date(2017, 1, 5));
    let Outcome::Answered(usd) = boc::parse_observations(&common::json(NOON, "IEXE0101-2016-12-28-2017-01-05.json"), "IEXE0101", Some(span)) else { panic!() };
    let series = boc::noon_series(Currency::USD);
    assert_eq!(series.len(), 1);
    assert_eq!(usd.description, series[0].description);
    // the 90-day forward shares the label USD_NOON, and its description is not the table's
    let Outcome::Answered(fwd) = boc::parse_observations(&common::json(NOON, "IEXE0105-2016-12-28-2017-01-05.json"), "IEXE0105", Some(span)) else { panic!() };
    assert_ne!(fwd.description, series[0].description);
    // the noon era stands to the day before the daily series began, 2017-01-02
    assert_eq!((series[0].first, series[0].last), (date(2007, 5, 1), date(2017, 1, 2)));
    // the zloty's daily series began long after the archive ended: the archive stands to its end
    let pln = boc::noon_series(cur("PLN"));
    assert_eq!(pln[0].last, date(2017, 4, 28));
    // two series for one currency hold days of their own
    let ars = boc::noon_series(cur("ARS"));
    assert_eq!(ars.iter().map(|s| (s.code, s.first, s.last)).collect::<Vec<_>>(), vec![("IEXE2702", date(2007, 5, 1), date(2010, 1, 1)), ("IEXE2703", date(2010, 1, 4), date(2017, 4, 28))]);
    let Outcome::Answered(plain) = boc::parse_observations(&common::json(NOON, "IEXE2703-2010-01-01-2010-01-08.json"), "IEXE2703", Some((date(2010, 1, 1), date(2010, 1, 8)))) else { panic!() };
    assert_eq!(plain.description, ars[1].description);
    // a currency with no noon series has none
    assert!(boc::noon_series(cur("SAR")).is_empty());
}

#[test]
fn every_noon_series_is_a_currency_and_the_eras_never_overlap() {
    for s in boc::NOON {
        assert!(Currency::parse(s.currency).is_ok(), "{}", s.currency);
        assert!(s.first <= s.last);
    }
    for c in boc::NOON.iter().map(|s| cur(s.currency)) {
        let series = boc::noon_series(c);
        assert!(series.windows(2).all(|w| w[0].last < w[1].first), "{c:?}");
        if let Some(a) = statcan::series_of(c) {
            assert!(statcan::ERA_END < series[0].first, "{c:?}: the archive's era overlaps the noon series");
            assert!(a.first <= statcan::ERA_END);
        }
    }
}

fn archive(c: &str) -> statcan::ArchiveSeries {
    statcan::series_of(cur(c)).unwrap()
}

#[test]
fn the_archive_states_business_days_and_nothing_for_weekends_and_holidays() {
    let span = (date(1998, 12, 28), date(1999, 1, 15));
    let Outcome::Answered(eur) = statcan::parse(&common::json(STATCAN, "v121742-1998-12-28-1999-01-15.json"), &archive("EUR"), span) else { panic!() };
    // the euro's first rate is 1999-01-04: the 1st is a holiday (no value) and the weekend states 0
    assert_eq!(eur.first().map(|r| r.0), Some(date(1999, 1, 4)));
    assert!(eur.iter().all(|(d, r)| d.weekday().to_monday_one_offset() <= 5 && *r > Dec::ZERO));
    let span = (date(1950, 10, 2), date(1950, 10, 10));
    let Outcome::Answered(gbp) = statcan::parse(&common::json(STATCAN, "v121720-1950-10-02-1950-10-10.json"), &archive("GBP"), span) else { panic!() };
    // eight decimals, exactly as written
    assert_eq!(gbp.first(), Some(&(date(1950, 10, 2), dec("2.96799999"))));
}

#[test]
fn a_wrong_archive_reply_writes_nothing() {
    let span = (date(2007, 4, 25), date(2007, 5, 4));
    let read = |name: &str| statcan::parse(&common::json(STATCAN, name), &archive("USD"), span);
    assert!(matches!(read("v121716-2007-04-25-2007-05-04.json"), Outcome::Answered(r) if r.len() == 8));
    assert!(matches!(read("wrong-shape-v121716-value-as-text.json"), Outcome::Mismatch(m) if m.path == "[0].object.vectorDataPoint[0].value"));
    assert!(matches!(read("wrong-meaning-v121716-zero-on-a-weekday.json"), Outcome::Meaning(w) if w.contains("2007-04-25")));
    assert!(matches!(read("wrong-meaning-v121716-another-vector.json"), Outcome::Meaning(w) if w.contains("121717")));
}

fn ymd(s: &str) -> Date {
    s.parse().unwrap()
}

#[test]
fn the_holiday_page_lists_each_closure_to_come() {
    let got = holidays::parse(&common::read(HOLIDAYS, "page-2026-09-24.html")).unwrap();
    let days: Vec<(Date, &str)> = got.iter().map(|(d, n)| (*d, n.as_str())).collect();
    assert_eq!(
        days,
        vec![
            (ymd("2026-09-30"), "National Day for Truth and Reconciliation"),
            (ymd("2026-10-12"), "Thanksgiving Day"),
            (ymd("2026-11-11"), "Remembrance Day"),
            (ymd("2026-12-25"), "Christmas Day"),
            // Boxing Day observed on the Monday, as the page states it
            (ymd("2026-12-28"), "Boxing Day"),
        ]
    );
    assert!(holidays::parse(&common::read(HOLIDAYS, "wrong-shape-no-date.html")).is_err());
    assert!(holidays::parse(&common::read(HOLIDAYS, "wrong-shape-no-closures.html")).is_err());
    // the page lists the closures still to come: read on a day after all of them,
    // it is last year's page, not current
    assert_eq!(holidays::current(&got, date(2026, 9, 24)), Ok(()));
    assert_eq!(holidays::current(&got, date(2026, 12, 28)), Ok(()));
    assert!(holidays::current(&got, date(2026, 12, 29)).unwrap_err().contains("not current"));
}
