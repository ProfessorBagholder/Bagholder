//! What a broker states beside its rows (`docs/plans/stage-3b-wealthsimple.md`,
//! "Statements and reads"): kept as stated, a later statement beside an earlier
//! one, the newest read.

mod common;

use std::collections::BTreeMap;

use bagholder_book::statements::{AccountDay, UnitsLine};
use bagholder_core::{Currency, Dec};
use common::*;

fn day(s: &str) -> jiff::civil::Date {
    s.parse().unwrap()
}

#[test]
fn a_day_stated_again_the_same_writes_nothing_and_differently_is_kept_beside_it() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let r1 = f.book.broker_read(f.connection, "history:tfsa-1", t0()).unwrap();
    let days = [AccountDay { day: day("2026-09-01"), net_value: cad("100"), net_deposits: cad("90") }, AccountDay { day: day("2026-09-02"), net_value: cad("101"), net_deposits: cad("90") }];
    assert!(f.book.store_account_days(a, &days, &r1).unwrap().is_empty());
    let r2 = f.book.broker_read(f.connection, "history:tfsa-1", at("2026-09-24T12:00:00Z")).unwrap();
    // the same again: nothing changed, nothing written
    assert!(f.book.store_account_days(a, &days, &r2).unwrap().is_empty());
    // the broker restates the second day
    let restated = [AccountDay { day: day("2026-09-02"), net_value: cad("102"), net_deposits: cad("90") }];
    assert_eq!(f.book.store_account_days(a, &restated, &r2).unwrap(), vec![day("2026-09-02")]);
    let stated = f.book.stated(a).unwrap();
    assert_eq!(stated.days[&day("2026-09-02")].0, cad("102"));
    assert_eq!(stated.days[&day("2026-09-01")].0, cad("100"));
    assert_eq!(f.book.last_account_day(a).unwrap(), Some(day("2026-09-02")));
}

#[test]
fn only_a_read_of_every_page_is_a_full_read() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    f.book.note_activity_read(a, at("2026-09-23T10:00:00Z"), true).unwrap();
    f.book.note_activity_read(a, at("2026-09-24T10:00:00Z"), false).unwrap();
    assert_eq!(f.book.activity_read_at(a).unwrap(), Some(at("2026-09-23T10:00:00Z")));
}

#[test]
fn the_newest_cash_and_units_are_the_ones_read() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let r = f.book.broker_read(f.connection, "balances", t0()).unwrap();
    f.book.store_cash(a, at("2026-09-23T10:00:00Z"), &BTreeMap::from([(Currency::CAD, d("5"))]), &r).unwrap();
    f.book.store_cash(a, at("2026-09-24T10:00:00Z"), &BTreeMap::from([(Currency::CAD, d("7")), (Currency::USD, d("-1"))]), &r).unwrap();
    let rec = f.store(&Spelled::v(1), "buy", &legs(vec![buy("tfsa-1", share("CA0000000001", "XYZ"), "3", "-30", "2026-09-01T15:00:00Z")]));
    let i = f.book.transactions_of(rec.record).unwrap()[0].instrument.unwrap();
    f.book.store_units(a, day("2026-09-22"), &[UnitsLine { instrument: i, quantity: d("3"), book_value: Some(cad("31")) }], &r, t0()).unwrap();
    let s = f.book.stated(a).unwrap();
    let (when, cash) = s.cash.unwrap();
    assert_eq!(when, at("2026-09-24T10:00:00Z"));
    assert_eq!(cash[&Currency::USD], d("-1"));
    let (as_of, units) = s.units.unwrap();
    assert_eq!((as_of, units[&i]), (day("2026-09-22"), Dec::parse("3").unwrap()));
}

#[test]
fn a_move_s_two_sides_are_linked_as_stated() {
    let f = Fixture::new();
    f.account(&["tfsa-1"]);
    f.account(&["rrsp-1"]);
    let out = f.store(&Spelled::v(1), "out", &legs(vec![sell("tfsa-1", share("CA0000000001", "XYZ"), "3", "0", "2026-09-01T15:00:00Z")]));
    let into = f.store(&Spelled::v(1), "in", &legs(vec![buy("rrsp-1", share("CA0000000001", "XYZ"), "3", "0", "2026-09-01T15:00:00Z")]));
    let o = f.book.transactions_of(out.record).unwrap()[0].id.clone();
    let i = f.book.transactions_of(into.record).unwrap()[0].id.clone();
    f.book.link_transfer(&o, &i).unwrap();
    assert_eq!(f.book.transfer_links().unwrap(), vec![(o, i)]);
}

#[test]
fn what_an_account_can_borrow_is_the_newest_statement_an_amount_or_why_not() {
    let f = Fixture::new();
    let a = f.account(&["margin-1"]);
    assert!(f.book.stated(a).unwrap().buying_power.is_none(), "nothing stated is nothing, not zero");
    let r = f.book.broker_read(f.connection, "balances", t0()).unwrap();
    f.book.store_buying_power(a, at("2026-09-23T10:00:00Z"), &Ok(cad("1500.25")), &r).unwrap();
    assert_eq!(f.book.stated(a).unwrap().buying_power, Some((at("2026-09-23T10:00:00Z"), Ok(cad("1500.25")))));
    f.book.store_buying_power(a, at("2026-09-24T10:00:00Z"), &Err("UnavailableSecurities (2 securities)".into()), &r).unwrap();
    assert_eq!(f.book.stated(a).unwrap().buying_power, Some((at("2026-09-24T10:00:00Z"), Err("UnavailableSecurities (2 securities)".to_string()))));
}

#[test]
fn a_part_s_last_read_is_the_newest_of_its_reads() {
    let f = Fixture::new();
    assert_eq!(f.book.last_read(f.connection, "accounts").unwrap(), None);
    f.book.broker_read(f.connection, "accounts", at("2026-09-23T20:00:00Z")).unwrap();
    f.book.broker_read(f.connection, "accounts", at("2026-09-24T20:00:00Z")).unwrap();
    f.book.broker_read(f.connection, "cash", at("2026-09-25T20:00:00Z")).unwrap();
    assert_eq!(f.book.last_read(f.connection, "accounts").unwrap(), Some(at("2026-09-24T20:00:00Z")));
}
