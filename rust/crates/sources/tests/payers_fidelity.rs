//! Fidelity Canada's own data, from recorded real replies (2026-09-24).

mod common;

use bagholder_core::jiff::civil::date;
use bagholder_core::{Currency, Dec};
use bagholder_sources::payers::fidelity::{self, Kind};
use bagholder_sources::payers::{newswire, Distribution};

#[test]
fn fidelity_states_each_series_schedule() {
    let funds = common::json("fidelity", "funds-trimmed.json");
    assert_eq!(fidelity::schedule_of(&funds, "FBTC").unwrap(), Some(Some(1)));
    assert_eq!(fidelity::schedule_of(&funds, "FCAB").unwrap(), Some(Some(12)));
    assert_eq!(fidelity::schedule_of(&funds, "ZZZQX").unwrap(), None);
}

#[test]
fn only_final_distribution_releases_are_read() {
    let kind = |t: &str| fidelity::kind_of(t);
    assert_eq!(kind("Fidelity Investments Canada ULC Announces Cash Distributions for Certain Fidelity ETFs and ETF Series of Fidelity Mutual Funds"), Some(Kind::Cash));
    assert_eq!(kind("Fidelity Investments Canada ULC Announces Final December 2025 Cash Distributions for Fidelity ETFs and ETF Series of Fidelity Mutual Funds"), Some(Kind::Cash));
    assert_eq!(kind("Fidelity Investments Canada ULC Announces Final 2025 Annual Reinvested Capital Gains Distributions for Fidelity ETFs and ETF Series of Fidelity Mutual Funds"), Some(Kind::AnnualReinvested));
    assert_eq!(kind("Fidelity Investments Canada ULC Announces Estimated December 2025 Cash Distributions for Fidelity ETFs and ETF Series of Fidelity Mutual Funds"), None);
    assert_eq!(kind("Fidelity Investments Canada ULC Announces Estimated 2025 Annual Reinvested Capital Gains Distributions for Fidelity ETFs and ETF Series of Fidelity Mutual Funds"), None);
    assert_eq!(kind("Fidelity Investments Canada ULC Announces Final Valuations and Special Reinvested Distributions for Terminating ETFs"), None);
    assert_eq!(kind("Fidelity Investments Canada ULC Announces Fund Closures to Streamline Offerings"), None);
}

fn release(name: &str) -> newswire::Release {
    newswire::release(&common::read("fidelity", name)).unwrap()
}

fn cad(ex: (i16, i8, i8), pay: (i16, i8, i8), cash: &str, reinvested: Option<&str>) -> Distribution {
    let ex = date(ex.0, ex.1, ex.2);
    Distribution { ex_date: ex, record_date: Some(ex), pay_date: Some(date(pay.0, pay.1, pay.2)), cash: Dec::parse(cash).unwrap(), reinvested: reinvested.map(|r| Dec::parse(r).unwrap()), currency: Currency::CAD }
}

#[test]
fn each_table_takes_the_record_and_pay_dates_of_its_own_sentence() {
    let r = release("release-2026-09-21-cash-trimmed.html");
    // an ETF, in the first table: record 2026-09-28, paid 2026-09-30
    assert_eq!(fidelity::rows_for(&r, Kind::Cash, "FCCD").unwrap(), (vec![cad((2026, 9, 28), (2026, 9, 30), "0.09773", None)], vec![]));
    // an ETF Series, in the second: record 2026-09-30, paid 2026-10-02; one row for both classes
    let fcab = vec![cad((2026, 9, 30), (2026, 10, 2), "0.11549", None)];
    assert_eq!(fidelity::rows_for(&r, Kind::Cash, "FCAB").unwrap().0, fcab);
    assert_eq!(fidelity::rows_for(&r, Kind::Cash, "FCAB.U").unwrap().0, fcab);
    // a fund that pays none this month (`-`), and one not in the release
    assert_eq!(fidelity::rows_for(&r, Kind::Cash, "FMPI").unwrap(), (vec![], vec![]));
    assert_eq!(fidelity::rows_for(&r, Kind::Cash, "ZZZQX").unwrap(), (vec![], vec![]));
    // a final December release states its one sentence before both tables
    let dec = release("release-2025-12-29-final-december-cash-trimmed.html");
    assert_eq!(fidelity::rows_for(&dec, Kind::Cash, "FCAB").unwrap().0, vec![cad((2025, 12, 29), (2025, 12, 31), "0.14081", None)]);
}

#[test]
fn the_annual_capital_gain_is_paid_in_units_on_the_day_its_release_states() {
    let r = release("release-2025-12-29-annual-trimmed.html");
    assert_eq!(fidelity::rows_for(&r, Kind::AnnualReinvested, "FBTC").unwrap(), (vec![cad((2025, 12, 29), (2025, 12, 31), "0", Some("0.84031"))], vec![]));
    // a fund with no capital gain this year (`-`)
    assert_eq!(fidelity::rows_for(&r, Kind::AnnualReinvested, "FCCD").unwrap(), (vec![], vec![]));
}

#[test]
fn an_amount_not_in_canadian_dollars_is_a_mismatch_naming_it() {
    let m = fidelity::rows_for(&release("wrong-shape-release-2026-09-21-amount-in-us-dollars.html"), Kind::Cash, "FCAB").unwrap_err();
    assert_eq!(m.path, "table 0");
    assert!(m.why.contains("US$"), "{m:?}");
}

#[test]
fn a_listed_release_carries_the_day_its_page_gives() {
    let listed = newswire::listed(&common::read("fidelity", "organization-page-1-trimmed.html")).unwrap();
    assert_eq!(listed.len(), 25);
    assert_eq!(listed[0].day, Some(date(2026, 9, 21)));
    assert!(listed.iter().all(|l| l.day.is_some()));
}
