//! Distribution releases as TMX keeps them (recorded 2026-10-23), each read for
//! one of its funds.

mod common;

use bagholder_core::jiff::civil::date;
use bagholder_core::Dec;
use bagholder_sources::payers::announced::{announced, Announced};

fn read(name: &str, ticker: &str) -> Option<Announced> {
    announced(&common::read("tmx", name), ticker).unwrap()
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

#[test]
fn harvest_states_the_month_s_days_in_one_sentence() {
    let a = read("story-plte.html", "PLTE").unwrap();
    assert_eq!(a, Announced { cash: dec("0.5000"), ex_date: date(2026, 10, 30), record_date: Some(date(2026, 10, 30)), pay_date: Some(date(2026, 11, 6)) });
}

#[test]
fn ninepoint_states_each_fund_s_days_in_its_row() {
    let a = read("story-cchi.html", "CCHI").unwrap();
    assert_eq!((a.cash, a.ex_date, a.pay_date), (dec("0.120000"), date(2026, 9, 29), Some(date(2026, 10, 6))));
}

#[test]
fn bmo_writes_the_amount_without_its_dollar_sign() {
    let a = read("story-zwc.html", "ZWC").unwrap();
    assert_eq!(a.cash, dec("0.120"));
    assert_eq!(a.record_date, Some(date(2026, 10, 29)));
    assert_eq!(a.ex_date, date(2026, 10, 29));
    assert_eq!(a.pay_date, Some(date(2026, 11, 3)));
}

#[test]
fn hamilton_anticipates_its_ex_date() {
    let a = read("story-hdiv.html", "HDIV").unwrap();
    assert_eq!((a.cash, a.ex_date), (dec("0.1950"), date(2026, 9, 29)));
}

#[test]
fn a_release_for_one_fund_states_its_amount_in_a_sentence() {
    let a = read("story-eit-un.html", "EIT.UN").unwrap();
    assert_eq!((a.cash, a.record_date, a.pay_date), (dec("0.10"), Some(date(2026, 10, 22)), Some(date(2026, 11, 13))));
}

#[test]
fn a_fund_the_release_does_not_name_is_none() {
    assert_eq!(read("story-plte.html", "ZZZQX"), None);
}
