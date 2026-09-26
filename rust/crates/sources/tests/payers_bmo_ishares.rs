//! BMO's and iShares Canada's own records, from recorded real replies (2026-09-24).

mod common;

use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec};
use bagholder_sources::outcome::Outcome;
use bagholder_sources::payers::{bmo, ishares_ca, Distribution};

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

#[test]
fn bmo_states_the_schedule_and_each_distributions_cash_and_reinvested_part() {
    let Outcome::Answered(zea) = bmo::parse(&common::json("bmo", "fund-ZEA.json"), "ZEA") else { panic!() };
    assert_eq!((zea.per_year, zea.rows.len()), (Some(4), 50));
    let year_end = zea.rows.iter().find(|r| r.ex_date == date(2025, 12, 30)).unwrap();
    assert_eq!((year_end.cash, year_end.reinvested), (dec("0.15"), Some(dec("0.12"))));
    let Outcome::Answered(zfl) = bmo::parse(&common::json("bmo", "fund-ZFL.json"), "ZFL") else { panic!() };
    assert_eq!(zfl.per_year, Some(12));
    assert!(matches!(bmo::parse(&common::json("bmo", "fund-ZZZQX.json"), "ZZZQX"), Outcome::NotCarried(_)));
    assert!(matches!(bmo::parse(&common::json("bmo", "wrong-meaning-fund-ZEA-parts-not-the-total.json"), "ZEA"), Outcome::Meaning(w) if w.contains("totals")));
}

#[test]
fn ishares_canada_states_each_row_with_its_currency_and_declared_rows_ahead() {
    let screener = common::json("ishares-canada", "screener-trimmed.json");
    let (id, path) = ishares_ca::fund_of(&screener, "XEQT").unwrap().unwrap();
    assert_eq!(id, "309480");
    assert!(path.starts_with("/ca/investors/en/products/309480/"));
    assert_eq!(ishares_ca::schedule_word(&common::read("ishares-canada", "page-309480.html")).as_deref(), Some("Quarterly"));
    let history = ishares_ca::json(common::read("ishares-canada", "history-309480.json").as_bytes()).unwrap();
    let Outcome::Answered(xeqt) = ishares_ca::parse_history(&history) else { panic!() };
    assert_eq!(xeqt[0], Distribution { ex_date: date(2026, 9, 24), record_date: Some(date(2026, 9, 24)), pay_date: Some(date(2026, 9, 29)), cash: dec("0.10200"), reinvested: None, currency: Currency::CAD });
    let year_end = xeqt.iter().find(|r| r.ex_date == date(2025, 12, 30)).unwrap();
    assert_eq!((year_end.cash, year_end.reinvested), (dec("0.20536"), Some(dec("0.32215"))));
    let xiu = ishares_ca::json(common::read("ishares-canada", "history-239832.json").as_bytes()).unwrap();
    assert!(matches!(ishares_ca::parse_history(&xiu), Outcome::Answered(r) if r.len() > 100));
    let wrong = ishares_ca::json(common::read("ishares-canada", "wrong-shape-history-amount-without-currency.json").as_bytes()).unwrap();
    assert!(matches!(ishares_ca::parse_history(&wrong), Outcome::Mismatch(_)));
}
