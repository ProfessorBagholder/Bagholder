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
    f.book.store_units(a, day("2026-09-22"), &[UnitsLine { instrument: i, quantity: d("3"), book_value: Some(cad("31")), value: Some(cad("36.75")) }], &r, t0()).unwrap();
    let s = f.book.stated(a).unwrap();
    let (when, cash) = s.cash.unwrap();
    assert_eq!(when, at("2026-09-24T10:00:00Z"));
    assert_eq!(cash[&Currency::USD], d("-1"));
    let (as_of, units) = s.units.unwrap();
    assert_eq!((as_of, units[&i]), (day("2026-09-22"), Dec::parse("3").unwrap()));
    assert_eq!(s.unit_values[&i], cad("36.75"), "what the broker states the units are worth");
}

#[test]
fn the_cash_a_full_read_covers_is_the_newest_stated_at_or_before_it() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let r = f.book.broker_read(f.connection, "balances", t0()).unwrap();
    // fractions of different lengths: ordered by the instant, not the text
    f.book.store_cash(a, at("2026-09-23T10:00:00.5Z"), &BTreeMap::from([(Currency::CAD, d("5"))]), &r).unwrap();
    f.book.store_cash(a, at("2026-09-23T10:00:00.450401Z"), &BTreeMap::from([(Currency::CAD, d("4"))]), &r).unwrap();
    f.book.store_cash(a, at("2026-09-24T10:00:00Z"), &BTreeMap::from([(Currency::CAD, d("7"))]), &r).unwrap();
    assert!(f.book.stated(a).unwrap().cash_read.is_none(), "no full read: no cash it covers");
    f.book.note_activity_read(a, at("2026-09-23T10:00:00.450401Z"), true).unwrap();
    let s = f.book.stated(a).unwrap();
    assert_eq!(s.cash.unwrap().1[&Currency::CAD], d("7"));
    let (when, cash) = s.cash_read.unwrap();
    assert_eq!((when, cash[&Currency::CAD]), (at("2026-09-23T10:00:00.450401Z"), d("4")));
    f.book.note_activity_read(a, at("2026-09-23T11:00:00Z"), true).unwrap();
    assert_eq!(f.book.stated(a).unwrap().cash_read.unwrap().1[&Currency::CAD], d("5"));
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

#[test]
fn a_statement_of_cash_keeps_what_the_live_records_held_as_it_was_stated() {
    use bagholder_core::hold::HoldKind;
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let m = Spelled::v(1);
    let order = f.store(&m, "order", &serde_json::json!({"hold": {"account": "tfsa-1", "kind": "buy", "currency": "USD", "amount": "70.00"}}));
    let r = f.book.broker_read(f.connection, "cash", t0()).unwrap();
    let first = at("2026-10-01T14:47:07Z");
    f.book.note_activity_read(a, first, true).unwrap();
    f.book.store_cash(a, first, &BTreeMap::from([(Currency::USD, d("14881.57"))]), &r).unwrap();
    let s = f.book.stated(a).unwrap();
    assert_eq!(s.cash_read_holds.len(), 1);
    let h = &s.cash_read_holds[0];
    assert_eq!((h.record, h.kind, h.currency, h.amount), (order.record, HoldKind::Buy, Some(Currency::USD), Some(d("70.00"))));

    // the order lapses: its record holds nothing, and the next statement none,
    // while the first keeps what it was stated beside
    let lapsed = f.book.store(&m, &f.incoming("order", r#"{"legs": []}"#), at("2026-10-02T14:09:55Z")).unwrap();
    assert_eq!(lapsed.record, order.record);
    let second = at("2026-10-02T14:09:55Z");
    f.book.note_activity_read(a, second, true).unwrap();
    f.book.store_cash(a, second, &BTreeMap::from([(Currency::USD, d("14951.57"))]), &r).unwrap();
    assert!(f.book.stated(a).unwrap().cash_read_holds.is_empty());
    let n: i64 = f.book.conn_for_tests().query_row("SELECT COUNT(*) FROM statement_holds", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn a_record_no_longer_live_holds_nothing_at_the_next_statement() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let order = f.store(&Spelled::v(1), "order", &serde_json::json!({"hold": {"account": "tfsa-1", "kind": "withdrawal", "currency": "CAD"}}));
    f.book.mark_removed(order.record, t0()).unwrap();
    let r = f.book.broker_read(f.connection, "cash", t0()).unwrap();
    f.book.note_activity_read(a, t0(), true).unwrap();
    f.book.store_cash(a, t0(), &BTreeMap::from([(Currency::CAD, d("1"))]), &r).unwrap();
    assert!(f.book.stated(a).unwrap().cash_read_holds.is_empty());
}

/// What the statement of cash a test makes holds for the record `r`.
fn held_at(f: &Fixture, a: bagholder_core::AccountId, when: &str) -> Vec<bagholder_core::hold::Hold> {
    let r = f.book.broker_read(f.connection, "cash", at(when)).unwrap();
    f.book.note_activity_read(a, at(when), true).unwrap();
    f.book.store_cash(a, at(when), &BTreeMap::from([(Currency::USD, d("1"))]), &r).unwrap();
    f.book.stated(a).unwrap().cash_read_holds
}

#[test]
fn a_working_buy_with_a_fill_against_its_order_holds_an_amount_its_row_does_not_state() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let m = Spelled::v(1);
    let pending = serde_json::json!({"hold": {"account": "tfsa-1", "kind": "buy", "currency": "USD", "amount": "500.00"}, "orders": ["order-a", "order-ext-a"]});
    f.store(&m, "order-a", &pending);
    // nothing filled against it: the row's amount is the hold
    assert_eq!(held_at(&f, a, "2026-10-01T15:00:00Z")[0].amount, Some(d("500.00")));

    // the app's own order of that id has a fill booked: the rest is unstated
    f.book
        .conn_for_tests()
        .execute(
            "INSERT INTO orders (id, broker, broker_account, broker_security, symbol, currency, side, order_type, quantity, time_in_force, request, created_at, state, filled, updated_at)
             VALUES ('order-ext-a', 'wealthsimple', 'tfsa-1', 'sec', 'XYZ', 'USD', 'buy', 'limit', '10', 'day', '{}', '2026-10-01T15:00:00Z', 'partly-filled', '4', '2026-10-01T15:00:00Z')",
            [],
        )
        .unwrap();
    assert_eq!(held_at(&f, a, "2026-10-01T16:00:00Z")[0].amount, None);
}

#[test]
fn a_working_buy_whose_order_has_another_record_that_moved_something_holds_an_unstated_amount() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let m = Spelled::v(1);
    f.store(&m, "order-b", &serde_json::json!({"hold": {"account": "tfsa-1", "kind": "buy", "currency": "USD", "amount": "300.00"}, "orders": ["order-b"]}));
    // a record of another order moves something: the hold is untouched
    let mut other = legs(vec![buy("tfsa-1", share("CA0000000001", "XYZ"), "3", "-90", "2026-10-01T15:00:00Z")]);
    other["orders"] = serde_json::json!(["order-other"]);
    f.store(&m, "fill-other", &other);
    assert_eq!(held_at(&f, a, "2026-10-01T15:30:00Z")[0].amount, Some(d("300.00")));
    // a fill of the same order, as its own record: the rest is unstated
    let mut fill = legs(vec![buy("tfsa-1", share("CA0000000001", "XYZ"), "1", "-30", "2026-10-01T15:00:00Z")]);
    fill["orders"] = serde_json::json!(["order-b"]);
    f.store(&m, "fill-b", &fill);
    assert_eq!(held_at(&f, a, "2026-10-01T16:00:00Z")[0].amount, None);
}
