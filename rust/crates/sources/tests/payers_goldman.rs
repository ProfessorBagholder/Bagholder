//! Goldman Sachs's own record, from recorded real replies (2026-09-24).

mod common;

use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec};
use bagholder_sources::payers::{self, goldman, Distribution, Record};

#[test]
fn goldman_states_each_class_its_schedule_and_every_distribution() {
    let c = goldman::class_of(&common::json("goldman-sachs", "funds-trimmed.json"), "GSWO").unwrap().unwrap();
    assert_eq!((c.pv_number.as_str(), c.share_class.as_str(), c.per_year, c.currency), ("PV104218", "38149W739", Some(4), Currency::USD));
    assert_eq!(goldman::class_of(&common::json("goldman-sachs", "funds-trimmed.json"), "GLOV").unwrap(), None);
    let rows = goldman::parse_detail(&common::json("goldman-sachs", "fund-GSWO.json"), "GSWO", Currency::USD).unwrap();
    assert_eq!(rows[0], Distribution { ex_date: date(2026, 6, 24), record_date: Some(date(2026, 6, 24)), pay_date: Some(date(2026, 6, 30)), cash: Dec::parse("0.3169").unwrap(), reinvested: None, currency: Currency::USD });
    // a date stated with no amount is no distribution
    assert!(rows.iter().all(|r| r.ex_date != date(2025, 12, 31)));
    // the page repeats 2025-12-23 identically: one row once checked
    let n = rows.iter().filter(|r| r.ex_date == date(2025, 12, 23)).count();
    assert_eq!(n, 2);
    let checked = payers::checked(Record { rows, per_year: Some(4), by_record: vec![] }, "2026-09-24T04:00:00Z".parse().unwrap()).unwrap();
    assert_eq!(checked.rows.iter().filter(|r| r.ex_date == date(2025, 12, 23)).count(), 1);
}
