//! Hamilton's and Purpose's own pages, from recorded real pages (2026-09-24).

mod common;

use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec};
use bagholder_sources::outcome::Outcome;
use bagholder_sources::payers::{hamilton, purpose, Distribution, Record};

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn answered(o: Outcome<Record>) -> Record {
    match o {
        Outcome::Answered(r) => r,
        other => panic!("{other:?}"),
    }
}

#[test]
fn hamilton_states_its_schedule_and_every_distribution_with_no_record_date() {
    let hmax = answered(hamilton::parse_page(&common::read("hamilton", "page-hmax.html"), Currency::CAD));
    assert_eq!((hmax.per_year, hmax.rows.len()), (Some(12), 43));
    assert_eq!(hmax.rows[0], Distribution { ex_date: date(2026, 8, 31), record_date: None, pay_date: Some(date(2026, 9, 8)), cash: dec("0.1690"), reinvested: None, currency: Currency::CAD });
    let cday = answered(hamilton::parse_page(&common::read("hamilton", "page-cday.html"), Currency::CAD));
    assert_eq!((cday.per_year, cday.rows.len()), (Some(24), 28));
    let amax = answered(hamilton::parse_page(&common::read("hamilton", "page-amax.html"), Currency::CAD));
    assert_eq!((amax.per_year, amax.rows.len()), (Some(12), 31));
    assert!(matches!(hamilton::parse_page(&common::read("hamilton", "wrong-shape-page-hmax-amount-not-a-number.html"), Currency::CAD), Outcome::Mismatch(m) if m.why.contains("soon")));
}

#[test]
fn purpose_carries_each_series_its_schedule_and_its_distributions() {
    let v = purpose::page_data(&common::read("purpose", "page-psa.html")).unwrap();
    let psa = answered(purpose::parse(&v, "PSA"));
    assert_eq!((psa.per_year, psa.rows.len()), (Some(12), 155));
    assert_eq!(psa.rows.last(), Some(&Distribution { ex_date: date(2026, 8, 27), record_date: Some(date(2026, 8, 27)), pay_date: Some(date(2026, 9, 2)), cash: dec("0.0866"), reinvested: None, currency: Currency::CAD }));
    assert!(matches!(purpose::parse(&v, "ZZZQX"), Outcome::NotCarried(_)));
    let wrong = purpose::page_data(&common::read("purpose", "wrong-shape-page-psa-unknown-kind.html")).unwrap();
    assert!(matches!(purpose::parse(&wrong, "PSA"), Outcome::Mismatch(m) if m.why.contains("Special")));
}
