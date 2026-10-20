//! Each outside account's balance by day (migration 36): a read of a span
//! replaces the days held for that account in that span and keeps the rest.

mod common;

use bagholder_book::debts::OutsideDay;
use bagholder_core::Currency;
use common::*;

fn day(s: &str) -> jiff::civil::Date {
    s.parse().unwrap()
}

fn days(xs: &[(&str, &str)]) -> Vec<OutsideDay> {
    xs.iter().map(|(dd, v)| OutsideDay { day: day(dd), amount: d(v), currency: Currency::CAD }).collect()
}

#[test]
fn a_read_of_a_span_replaces_that_span_and_keeps_the_days_outside_it() {
    let f = Fixture::new();
    // the whole history, stated weekly
    f.book.keep_outside_days("loc", day("2014-01-01"), day("2026-10-20"), &days(&[("2026-08-03", "500"), ("2026-09-07", "520"), ("2026-10-12", "540")]), t0()).unwrap();
    f.book.keep_outside_days("home", day("2014-01-01"), day("2026-10-20"), &days(&[("2026-01-05", "300000")]), t0()).unwrap();
    // the last month, stated daily: the weekly day in it is replaced
    f.book.keep_outside_days("loc", day("2026-09-20"), day("2026-10-20"), &days(&[("2026-10-18", "545"), ("2026-10-19", "550")]), at("2026-10-20T12:00:00Z")).unwrap();
    let got = f.book.outside_days().unwrap();
    let loc: Vec<(String, String)> = got["loc"].iter().map(|x| (x.day.to_string(), x.amount.to_string())).collect();
    assert_eq!(loc, [("2026-08-03", "500"), ("2026-09-07", "520"), ("2026-10-18", "545"), ("2026-10-19", "550")].map(|(a, b)| (a.to_string(), b.to_string())));
    assert_eq!(got["home"].len(), 1, "another account's days are its own");
}

#[test]
fn a_day_outside_the_span_asked_is_not_kept() {
    let f = Fixture::new();
    f.book.keep_outside_days("loc", day("2026-10-01"), day("2026-10-20"), &days(&[("2026-09-30", "1"), ("2026-10-01", "2")]), t0()).unwrap();
    assert_eq!(f.book.outside_days().unwrap()["loc"].len(), 1);
}
