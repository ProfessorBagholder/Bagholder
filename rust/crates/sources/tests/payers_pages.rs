//! Fund companies' own pages, read from recorded real pages (2026-09-24): the
//! schedule each states and every distribution it lists, exactly as written.

mod common;

use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec};
use bagholder_sources::outcome::Outcome;
use bagholder_sources::payers::{evolve, harvest, Distribution, Record};

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
fn evolve_states_twice_a_month_and_lists_announced_distributions() {
    let easy = answered(evolve::parse_page(&common::read("evolve", "page-easy.html"), Currency::CAD));
    assert_eq!(easy.per_year, Some(24));
    assert_eq!(easy.rows.len(), 14);
    assert_eq!(easy.rows[0], Distribution { ex_date: date(2026, 3, 31), record_date: Some(date(2026, 3, 31)), pay_date: Some(date(2026, 4, 8)), cash: dec("0.31000"), reinvested: None, currency: Currency::CAD });
    // announced ahead, on the page on 2026-09-24
    assert!(easy.rows.iter().any(|r| r.ex_date == date(2026, 10, 15)));
    let sixy = answered(evolve::parse_page(&common::read("evolve", "page-sixy.html"), Currency::CAD));
    assert_eq!(sixy.per_year, Some(24));
    assert!(sixy.rows.iter().any(|r| r.ex_date == date(2025, 12, 31)), "the earlier year's table is read too");
}

#[test]
fn harvest_states_its_schedule_in_either_layout() {
    let hhis = answered(harvest::parse_page(&common::read("harvest", "page-hhis.html"), Currency::CAD));
    assert_eq!(hhis.per_year, Some(12));
    assert_eq!(hhis.rows.len(), 19);
    assert_eq!(hhis.rows[0], Distribution { ex_date: date(2026, 8, 31), record_date: Some(date(2026, 8, 31)), pay_date: Some(date(2026, 9, 4)), cash: dec("0.2700"), reinvested: None, currency: Currency::CAD });
    assert_eq!(hhis.rows.last().map(|r| r.ex_date), Some(date(2025, 2, 28)));
    let hbix = answered(harvest::parse_page(&common::read("harvest", "page-hbix.html"), Currency::CAD));
    assert_eq!((hbix.per_year, hbix.rows.len()), (Some(12), 16));
    assert!(hbix.rows.iter().any(|r| r.ex_date == date(2025, 12, 31) && r.cash == dec("0.2400")), "the earlier year's table");
    let hpyb = answered(harvest::parse_page(&common::read("harvest", "page-hpyb.html"), Currency::CAD));
    assert_eq!((hpyb.per_year, hpyb.rows.len()), (Some(24), 15));
    for (page, n) in [("page-plte.html", 19), ("page-rddy.html", 11)] {
        let r = answered(harvest::parse_page(&common::read("harvest", page), Currency::CAD));
        assert_eq!((r.per_year, r.rows.len()), (Some(12), n), "{page}");
    }
}

#[test]
fn a_wrong_page_writes_nothing() {
    use bagholder_sources::payers;
    assert!(matches!(evolve::parse_page(&common::read("evolve", "wrong-shape-page-easy-schedule-unknown.html"), Currency::CAD), Outcome::Mismatch(m) if m.why.contains("Every so often")));
    let hhis = answered(harvest::parse_page(&common::read("harvest", "wrong-meaning-page-hhis-paid-before-ex.html"), Currency::CAD));
    assert!(payers::checked(hhis, "2026-09-24T04:00:00Z".parse().unwrap()).unwrap_err().contains("paid 2026-08-04"));
}
