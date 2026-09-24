//! TMX's recorded replies read only by the shape check elsewhere: QUU's
//! distributions and the TSX 60's first sessions, each read exactly, asked and
//! parsed.

mod common;

use std::sync::Arc;

use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::{Currency, Dec};
use bagholder_sources::adapters::tmx::{self, TmxDistribution};
use bagholder_sources::outcome::Outcome;

const TMX: &str = "tmx";
const URL: &str = "https://app-money.tmx.com/graphql";

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn row(ex: Date, record: Option<Date>, pay: Option<Date>, cash: &str, in_units: Option<&str>) -> TmxDistribution {
    TmxDistribution { ex_date: ex, record_date: record, pay_date: pay, cash: dec(cash), in_units: in_units.map(dec), currency: Currency::CAD }
}

#[test]
fn quu_lists_cash_rows_year_end_units_and_a_year_end_of_nothing() {
    let Outcome::Answered(rows) = tmx::parse_dividends(&common::json(TMX, "dividends-QUU.json"), "QUU") else { panic!() };
    assert_eq!(rows.len(), 39);
    assert_eq!(rows[0], row(date(2026, 9, 21), Some(date(2026, 9, 21)), Some(date(2026, 9, 28)), "0.91304", None));
    assert_eq!(rows[38], row(date(2018, 3, 19), Some(date(2018, 3, 20)), Some(date(2018, 3, 27)), "0.07273", None));
    let on = |d: Date| rows.iter().find(|r| r.ex_date == d).copied().unwrap();
    // the year-end notice of nothing
    assert_eq!(on(date(2025, 12, 31)), row(date(2025, 12, 31), Some(date(2025, 12, 31)), None, "0", None));
    // undated year-ends: paid in units
    assert_eq!(on(date(2023, 12, 28)), row(date(2023, 12, 28), None, None, "0", Some("0.04676")));
    assert_eq!(on(date(2021, 12, 30)), row(date(2021, 12, 30), None, None, "0", Some("0.43507")));
    assert_eq!(on(date(2021, 3, 22)), row(date(2021, 3, 22), None, None, "0", Some("0.31301")));
    assert_eq!(on(date(2018, 12, 31)), row(date(2018, 12, 31), None, None, "0", Some("1.11806")));
    assert_eq!(rows.iter().filter(|r| r.in_units.is_some()).count(), 4);
}

#[test]
fn quu_is_asked_on_one_page_when_the_page_is_not_full() {
    let recorded = Arc::new(common::Recorded::new().with_body(URL, "getDividendsForSymbol", 200, TMX, "dividends-QUU.json"));
    let net = common::net(&recorded, "2026-09-24T04:00:00Z");
    let noted = tmx::ask_dividends(&net, "QUU");
    assert_eq!(noted.shape_change, None);
    let Outcome::Answered(rows) = noted.outcome else { panic!("{:?}", noted.outcome) };
    assert_eq!(rows, match tmx::parse_dividends(&common::json(TMX, "dividends-QUU.json"), "QUU") { Outcome::Answered(r) => r, other => panic!("{other:?}") });
    assert_eq!(recorded.asked.lock().unwrap().len(), 1);
}
