//! YieldMax's and Defiance's own pages, from recorded real pages (2026-09-24).

mod common;

use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec};
use bagholder_sources::outcome::Outcome;
use bagholder_sources::payers::{self, us_pages, Distribution, Record};

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

#[test]
fn yieldmax_states_a_weekly_schedule_and_rows_as_paid_at_the_time() {
    let schedule = us_pages::yieldmax_schedule(&common::json("yieldmax", "etf-msty.json"), &common::json("yieldmax", "distribution-frequency.json")).unwrap();
    assert_eq!(schedule, Some(Some(52)));
    assert_eq!(us_pages::yieldmax_schedule(&common::json("yieldmax", "etf-zzzqx.json"), &common::json("yieldmax", "distribution-frequency.json")).unwrap(), None);
    let Outcome::Answered(rows) = us_pages::yieldmax_page(&common::read("yieldmax", "page-msty.html")) else { panic!() };
    assert_eq!(rows[0], Distribution { ex_date: date(2026, 9, 24), record_date: Some(date(2026, 9, 24)), pay_date: Some(date(2026, 9, 25)), cash: dec("0.3421"), reinvested: None, currency: Currency::USD });
    // not restated for the 1:5 consolidation of 2025-12-08
    assert!(rows.iter().any(|r| r.ex_date == date(2025, 12, 4) && r.cash == dec("0.1388")));
    // the page repeats rows and carries a record date ten years out: each repeat is one
    // row, and that one date alone is not taken
    let checked = payers::checked(Record { form: bagholder_core::distribution::Form::Stated, rows, per_year: Some(52), by_record: vec![] }, "2026-09-24T04:00:00Z".parse().unwrap()).unwrap();
    assert_eq!(checked.rows.len(), 69);
    let slip = checked.rows.iter().find(|r| r.ex_date == date(2026, 7, 30)).unwrap();
    assert_eq!((slip.record_date, slip.pay_date, slip.cash), (None, Some(date(2026, 7, 31)), dec("0.2222")));
}

#[test]
fn defiance_lists_distributions_and_scheduled_dates_with_no_amount() {
    let Outcome::Answered(rows) = us_pages::defiance_page(&common::read("defiance", "page-qtum.html")) else { panic!() };
    assert!(rows.iter().all(|r| r.ex_date != date(2026, 12, 23)), "a scheduled date with no amount is no distribution");
    assert!(rows.iter().any(|r| r.ex_date.year() == 2019));
    // a record date a day before its ex-date (June 2022) is within its distribution's week
    assert!(payers::checked(Record { form: bagholder_core::distribution::Form::Stated, rows, per_year: None, by_record: vec![] }, "2026-09-24T04:00:00Z".parse().unwrap()).is_ok());
}
