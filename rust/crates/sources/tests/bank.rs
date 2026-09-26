//! The Bank of Canada's rates and holidays, read from recorded real replies:
//! exactly what each answer states, and each wrong-shaped or wrong-meaning copy
//! refused with nothing taken from it.

mod common;

use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::{Currency, Dec};
use bagholder_sources::adapters::{boc, holidays, statcan};
use bagholder_sources::outcome::{Outcome, OutcomeKind};
use bagholder_sources::reply::Node;

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

/// The span a capture was asked for, from its name (`IEXE0101-2016-12-28-2017-01-05.json`).
fn captured_span(name: &str) -> (String, (Date, Date)) {
    let stem = name.trim_end_matches(".json");
    let (code, days) = stem.split_once('-').unwrap();
    let (from, to) = (&days[..10], &days[11..]);
    (code.to_string(), (ymd(from), ymd(to)))
}

#[test]
fn every_noon_table_row_a_recorded_reply_states_is_the_banks_own() {
    let mut checked = Vec::new();
    for name in common::answers(NOON, "IEXE") {
        let (code, span) = captured_span(&name);
        let Outcome::Answered(o) = boc::parse_observations(&common::json(NOON, &name), &code, Some(span)) else { panic!("{name}") };
        match boc::NOON.iter().find(|s| s.code == code) {
            Some(row) => {
                assert_eq!(o.description, row.description, "{name}");
                checked.push(code);
            }
            // the 90-day forward is recorded to show it is not a spot rate
            None => assert_eq!((code.as_str(), o.description.as_str()), ("IEXE0105", "U.S. dollar noon, 90-day")),
        }
    }
    assert_eq!(checked, vec!["IEXE0101", "IEXE2702", "IEXE2703", "IEXE4701"]);
    // the old cedi's whole series is the table's first and last day
    let ghc = boc::noon_series(cur("GHC"));
    let Outcome::Answered(o) = boc::parse_observations(&common::json(NOON, "IEXE4701-2007-05-01-2007-06-29.json"), "IEXE4701", Some((ghc[0].first, ghc[0].last))) else { panic!() };
    assert_eq!((o.rates.first().map(|r| r.0), o.rates.last().map(|r| r.0)), (Some(date(2007, 5, 1)), Some(date(2007, 6, 29))));
    assert_eq!(o.rates.len(), 43);
    // the dollar's noon series states 2016-12-30, its last day before the daily series,
    // and goes on to 2017-01-05: the table cuts it, not the Bank
    let Outcome::Answered(usd) = boc::parse_observations(&common::json(NOON, "IEXE0101-2016-12-28-2017-01-05.json"), "IEXE0101", Some((date(2016, 12, 28), date(2017, 1, 5)))) else { panic!() };
    assert_eq!(usd.rates.iter().map(|r| r.0).filter(|d| *d < boc::DAILY_LAUNCH).last(), Some(date(2016, 12, 30)));
    assert_eq!(usd.rates.iter().find(|r| r.0 == date(2017, 1, 3)), Some(&(date(2017, 1, 3), dec("1.3438"))));
}

#[test]
fn the_local_peso_goes_on_beside_the_plain_one_and_the_table_stands_it_only_to_2010_01_01() {
    let span = (date(2010, 1, 1), date(2010, 1, 8));
    let Outcome::Answered(local) = boc::parse_observations(&common::json(NOON, "IEXE2702-2010-01-01-2010-01-08.json"), "IEXE2702", Some(span)) else { panic!() };
    let Outcome::Answered(plain) = boc::parse_observations(&common::json(NOON, "IEXE2703-2010-01-01-2010-01-08.json"), "IEXE2703", Some(span)) else { panic!() };
    // both state 2010-01-04 to 2010-01-08, and differ
    assert_eq!(local.rates.iter().map(|r| r.0).collect::<Vec<_>>(), plain.rates.iter().map(|r| r.0).collect::<Vec<_>>());
    assert_eq!(local.rates[0], (date(2010, 1, 4), dec("0.2690")));
    assert_eq!(plain.rates[0], (date(2010, 1, 4), dec("0.2734")));
    assert_eq!(local.rates[4], (date(2010, 1, 8), dec("0.2671")));
    assert_eq!(plain.rates[4], (date(2010, 1, 8), dec("0.2722")));
    // asked over the table's span for it, those days are outside what was asked
    let ars = boc::noon_series(cur("ARS"));
    let asked = boc::parse_observations(&common::json(NOON, "IEXE2702-2010-01-01-2010-01-08.json"), "IEXE2702", Some((ars[0].first, ars[0].last)));
    assert!(matches!(asked, Outcome::Meaning(w) if w == "IEXE2702 answered 2010-01-04, outside the 2007-05-01 to 2010-01-01 asked"));
}

#[test]
fn every_archive_row_a_recorded_reply_states_is_statistics_canadas_own() {
    // the vector and coordinate each capture names are the table's (a reply naming
    // others is a meaning failure), and its first rate is the table's first day
    let mut checked = Vec::new();
    for name in common::answers(STATCAN, "v") {
        let (vector, span) = captured_span(&name);
        let row = statcan::ARCHIVE.iter().find(|a| format!("v{}", a.vector) == vector).unwrap_or_else(|| panic!("{name}: no row"));
        let Outcome::Answered(rates) = statcan::parse(&common::json(STATCAN, &name), row, span) else { panic!("{name}") };
        if span.0 <= row.first {
            assert_eq!(rates.first().map(|r| r.0), Some(row.first), "{name}");
        }
        checked.push(row.currency);
    }
    checked.sort_unstable();
    // every row's first day is checked against a capture reaching back before it
    let mut all: Vec<&str> = statcan::ARCHIVE.iter().map(|a| a.currency).collect();
    all.sort_unstable();
    assert_eq!(checked, all);
    // the dollar's archive states 2007-04-30, its era's last day, and 2007-05-01, the
    // noon series' first: the era ends where the table says
    let Outcome::Answered(usd) = statcan::parse(&common::json(STATCAN, "v121716-2007-04-25-2007-05-04.json"), &archive("USD"), (date(2007, 4, 25), date(2007, 5, 4))) else { panic!() };
    assert_eq!(usd.iter().find(|r| r.0 == statcan::ERA_END), Some(&(date(2007, 4, 30), dec("1.1067"))));
    assert_eq!(usd.iter().find(|r| r.0 == date(2007, 5, 1)), Some(&(date(2007, 5, 1), dec("1.1089"))));
    assert_eq!(statcan::ERA_END.tomorrow().unwrap(), boc::noon_series(Currency::USD)[0].first);
    // asked over its era, the days after it are outside what was asked
    let era = statcan::parse(&common::json(STATCAN, "v121716-2007-04-25-2007-05-04.json"), &archive("USD"), (date(2007, 4, 25), statcan::ERA_END));
    assert!(matches!(era, Outcome::Meaning(w) if w == "vector 121716 answered 2007-05-01, outside the 2007-04-25 to 2007-04-30 asked"));
}

#[test]
fn the_daily_series_begins_on_its_launch_day_whatever_span_is_asked() {
    let Outcome::Answered(o) = boc::parse_observations(&common::json(BOC, "FXUSDCAD-2016-12-28-2017-01-05.json"), "FXUSDCAD", Some((date(2016, 12, 28), date(2017, 1, 5)))) else { panic!() };
    assert_eq!(o.rates, vec![(date(2017, 1, 3), dec("1.3435")), (date(2017, 1, 4), dec("1.3315")), (date(2017, 1, 5), dec("1.3244"))]);
    assert_eq!(o.rates[0].0, boc::DAILY_LAUNCH);
    assert_eq!(boc::daily_description(&o.description), Ok(false));
    assert_eq!(boc::noon_series(Currency::USD)[0].last.tomorrow().unwrap(), boc::DAILY_LAUNCH);
}

#[test]
fn a_rate_that_is_not_a_decimal_is_a_mismatch() {
    let read = boc::parse_observations(&common::json(BOC, "wrong-shape-FXUSDCAD-rate-not-a-decimal.json"), "FXUSDCAD", Some((date(2026, 8, 28), date(2026, 9, 8))));
    assert!(matches!(read, Outcome::Mismatch(m) if m.path == "observations[0].FXUSDCAD.v"));
}

#[test]
fn a_noon_reply_describing_another_series_or_missing_its_description_is_refused() {
    let span = (date(2007, 5, 1), date(2017, 1, 2));
    let Outcome::Answered(fwd) = boc::parse_observations(&common::json(NOON, "wrong-meaning-IEXE0101-forward-rate.json"), "IEXE0101", Some(span)) else { panic!() };
    assert_ne!(fwd.description, boc::noon_series(Currency::USD)[0].description);
    let ghc = (date(2007, 5, 1), date(2007, 6, 29));
    assert!(matches!(boc::parse_observations(&common::json(NOON, "wrong-shape-IEXE4701-no-description.json"), "IEXE4701", Some(ghc)), Outcome::Mismatch(m) if m.path == "seriesDetail.IEXE4701.description"));
}

#[test]
fn last_years_holiday_page_is_not_current() {
    let got = holidays::parse(&common::read(HOLIDAYS, "wrong-meaning-last-years-dates.html")).unwrap();
    assert_eq!(got.iter().map(|c| c.0).collect::<Vec<_>>(), vec![ymd("2025-09-30"), ymd("2025-10-12"), ymd("2025-11-11"), ymd("2025-12-25"), ymd("2025-12-28")]);
    assert!(holidays::current(&got, date(2026, 9, 24)).unwrap_err().contains("not current"));
}

#[test]
fn the_noon_table_is_the_banks_series_list_and_leaves_out_only_what_is_not_a_noon_spot_rate() {
    // the Bank's own list of its series (`valet/lists/series/json`), its IEXE series and the table's kept
    let list = common::json(NOON, "series-list-noon-trimmed.json");
    let series = Node::root(&list).obj("series").unwrap();
    let text = |code: &str, field: &str| series.obj(code).and_then(|s| s.text(field).map(str::to_string)).ok();
    for row in boc::NOON {
        assert_eq!(text(row.code, "description").as_deref(), Some(row.description), "{}", row.code);
        assert_eq!(text(row.code, "label"), Some(format!("{}_NOON", row.currency)), "{}", row.code);
    }
    // what the table leaves out: the reciprocal, the close, high and low, the 90-day
    // forward and its spreads, each named for what it is
    let mut left: Vec<&str> = series.keys().unwrap().into_iter().filter(|c| !boc::NOON.iter().any(|r| r.code == *c)).collect();
    left.sort_unstable();
    assert_eq!(
        left,
        vec![
            "IEXE0101_RECIPROCAL", "IEXE0102", "IEXE0103", "IEXE0104", "IEXE0105", "IEXE0106", "IEXE0124", "IEXE0125", "IEXE0301.CL", "IEXE0701.CL", "IEXE0901.CL", "IEXE1001.CL", "IEXE1101.CL", "IEXE1201.CL",
            "IEXE1401.CL", "IEXE1601.CL", "IEXE1901.CL", "IEXE2001.CL",
        ]
    );
    for code in &left {
        let d = text(code, "description").unwrap_or_default();
        assert!(["(close)", "(high)", "(low)", "90-day", "forward points", "(noon)"].iter().any(|w| d.contains(w)), "{code}: {d}");
    }
    assert_eq!(text("IEXE0101_RECIPROCAL", "label").as_deref(), Some("CAD/USD Noon Rate"));
}

#[test]
fn the_archive_table_is_statistics_canadas_series_information() {
    // `getSeriesInfoFromVector` for the twelve vectors of table 10-10-0008
    let info = common::json(STATCAN, "series-info-all.json");
    let items = Node::root(&info).as_list().unwrap();
    assert_eq!(items.len(), statcan::ARCHIVE.len());
    let names = [("USD", "United States dollar"), ("NOK", "Norwegian krone"), ("SEK", "Swedish krona"), ("CHF", "Swiss franc"), ("GBP", "United Kingdom pound sterling"), ("AUD", "Australian dollar"), ("HKD", "Hong Kong dollar"), ("NZD", "New Zealand dollar"), ("MXN", "Mexican pesos"), ("EUR", "European euro"), ("DKK", "Danish krone"), ("JPY", "Japanese yen")];
    for row in statcan::ARCHIVE {
        let o = items.iter().map(|i| i.obj("object").unwrap()).find(|o| o.int("vectorId").ok() == Some(i64::from(row.vector))).unwrap_or_else(|| panic!("vector {}", row.vector));
        assert_eq!(o.text("coordinate").ok(), Some(row.coordinate), "{}", row.currency);
        assert_eq!(o.int("productId").ok(), Some(10100008), "{}", row.currency);
        let name = names.iter().find(|(c, _)| *c == row.currency).map(|(_, n)| *n).expect("named");
        assert_eq!(o.text("SeriesTitleEn").ok().map(str::to_string), Some(format!("Canada;{name}, noon spot rate")), "{}", row.currency);
    }
}
