//! A past day's holdings (`SPEC.md` §4 Portfolio, A past range) are what the
//! positions read as of that day states (brief 79 §2.4), worked out once and kept
//! while a screen holds them: after any change they equal a fresh working-out, and
//! the engine says they moved exactly when a change reached them: that day's read,
//! a close or rate of it or before; never a quote, a later close, or another day's
//! read.

mod common;

use std::collections::BTreeMap;

use serde_json::{json, Value};

use bagholder_core::jiff::civil::Date;
use bagholder_core::{Dec, Money};
use bagholder_engine::engine::Entity;
use bagholder_engine::input::{Quote, QuoteSource};
use bagholder_engine::{Change, Engine};
use common::*;

/// An account holding X on a weekend day as its positions read states it, and X's
/// closes about it; the journal holds nothing of it.
fn case() -> Value {
    json!({
        "name": "a past day's holdings",
        "today": "2026-03-02",
        "accounts": [{"id": "A"}],
        "instruments": [{"id": "X", "symbol": "XQA", "currency": "CAD"}],
        "closes": {"X": {"2026-02-05": "12", "2026-02-06": "13"}},
        "brokers": {
            "A": {
                "cash": {"CAD": "50"}, "as_of": "2026-03-02T15:00:00Z", "net_value": {"2026-02-06": "1350"},
                "days_held": {"2026-02-08": {"held": {"X": "100"}, "book": {"X": "1000 CAD"}, "cash": {"CAD": "50"}}, "2026-02-09": {"held": {"X": "100"}, "book": {"X": "1000 CAD"}}}
            }
        },
        "transactions": []
    })
}

#[test]
fn a_past_days_holdings_are_its_read_and_move_only_with_what_they_are_worked_out_from() {
    let mut b = build(&case());
    let mut e = engine(&mut b);
    let day: Date = "2026-02-08".parse().unwrap();
    let x = b.ids.instrument("X");
    let a = b.ids.account("A");
    let held = e.past(day);
    // what that day's read states, at that day's last close
    let p = held.positions.iter().find(|p| p.instrument == x).expect("the read's holding");
    assert_eq!((p.qty.clone().unwrap(), p.book.clone().unwrap().amount, p.mark.as_ref().unwrap().price), (Dec::from_int(100), Dec::from_int(1000), Dec::from_int(13)));
    let fresh = |e: &Engine| Engine::build(e.inputs().clone()).past(day);
    let check = |e: &Engine, what: &str| assert_eq!(*e.past(day), *fresh(e), "{what}: the kept holdings differ from a fresh working-out");

    // a quote now says nothing of that day
    let moved = e.apply(Change::Quote(x, Some(Quote { price: Money::new(Dec::from_int(99), p.currency), change: None, change_pct: None, at: None, source: QuoteSource::Listing })));
    assert!(!moved.0.contains_key(&Entity::Past), "a quote moved a past day");
    check(&e, "after a quote");

    // a close after that day says nothing of it either
    let mut closes: BTreeMap<Date, Money> = e.inputs().market.closes[&x].clone();
    closes.insert("2026-02-20".parse().unwrap(), Money::new(Dec::from_int(15), p.currency));
    let moved = e.apply(Change::Closes(x, closes.clone()));
    assert!(!moved.0.contains_key(&Entity::Past), "a later close moved a past day");
    check(&e, "after a later close");

    // a close on or before it does, and the kept holdings are worked out again
    closes.insert("2026-02-07".parse().unwrap(), Money::new(Dec::from_int(14), p.currency));
    let moved = e.apply(Change::Closes(x, closes));
    assert!(moved.0.contains_key(&Entity::Past), "a close of that day did not move it");
    check(&e, "after that day's close");
    let again = e.past(day);
    assert_eq!(again.positions.iter().find(|p| p.instrument == x).unwrap().mark.as_ref().unwrap().price, Dec::from_int(14));

    // another day's read says nothing of it; that day's own does
    let mut stated = e.inputs().market.brokers[&a].clone();
    stated.days_held.get_mut(&"2026-02-09".parse().unwrap()).unwrap().held.insert(x, Dec::from_int(80));
    let moved = e.apply(Change::Broker(a, Some(stated.clone())));
    assert!(!moved.0.contains_key(&Entity::Past), "a later day's read moved a past day");
    check(&e, "after a later day's read");
    stated.days_held.get_mut(&day).unwrap().held.insert(x, Dec::from_int(60));
    let moved = e.apply(Change::Broker(a, Some(stated)));
    assert!(moved.0.contains_key(&Entity::Past), "that day's read did not move it");
    check(&e, "after that day's read");
    assert_eq!(e.past(day).positions.iter().find(|p| p.instrument == x).unwrap().qty.clone().unwrap(), Dec::from_int(60));

    // nothing held: nothing to say moved
    drop(again);
    drop(held);
    let mut stated = e.inputs().market.brokers[&a].clone();
    stated.days_held.get_mut(&day).unwrap().held.insert(x, Dec::from_int(70));
    assert!(!e.apply(Change::Broker(a, Some(stated))).0.contains_key(&Entity::Past), "a day no screen holds was said to move");
}

/// Dates ending on that day show its read's holdings: held then, whenever the
/// journal opened them, they are in the dates however they begin, and none is
/// left out for the journal holding no lot of it.
#[test]
fn dates_ending_on_a_past_day_show_the_holdings_its_read_states() {
    let mut b = build(&case());
    let e = Engine::build(b.inputs.clone());
    let x = b.ids.instrument("X");
    let day: Date = "2026-02-08".parse().unwrap();
    for from in [None, Some(day), Some("2026-01-01".parse().unwrap())] {
        let f = bagholder_engine::scope::Filters { dates: bagholder_engine::scope::Dates::Range { from, to: Some(day) }, ..Default::default() };
        let pf = e.holdings(&f);
        let held: Vec<_> = pf.positions.iter().map(|i| pf.held(&[])[*i].instrument).collect();
        assert_eq!(held, [x], "from {from:?}");
    }
}

/// A past day no read states is not yet read: the engine names it to be read
/// while a screen holds it, and stops once its read is there.
#[test]
fn a_past_day_no_read_states_is_named_to_be_read() {
    let mut b = build(&case());
    let mut e = engine(&mut b);
    let day: Date = "2026-02-10".parse().unwrap();
    let a = b.ids.account("A");
    let held = e.past(day);
    assert!(held.positions.is_empty());
    assert!(e.needs().holdings_on.contains(&day), "{:?}", e.needs().holdings_on);
    let mut stated = e.inputs().market.brokers[&a].clone();
    stated.days_held.insert(day, Default::default());
    e.apply(Change::Broker(a, Some(stated)));
    assert!(!e.needs().holdings_on.contains(&day));
    drop(held);
}
