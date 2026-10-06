//! A past day's holdings (`SPEC.md` §4 Portfolio, A past range) are worked out once
//! and kept while a screen holds them: after any change they equal a fresh working-out,
//! and the engine says they moved exactly when a change reached them, never on a quote
//! or a close or rate after that day.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use bagholder_core::jiff::civil::Date;
use bagholder_core::{Dec, Money};
use bagholder_engine::engine::Entity;
use bagholder_engine::input::{Quote, QuoteSource};
use bagholder_engine::{Change, Engine};
use common::*;

fn case(name: &str) -> Value {
    let all: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases/past_range.json")).unwrap()).unwrap();
    all.into_iter().find(|c| c["name"].as_str().unwrap().contains(name)).unwrap()
}

#[test]
fn a_past_days_holdings_move_only_with_what_they_are_worked_out_from() {
    let mut b = build(&case("weekend"));
    let mut e = engine(&mut b);
    let day: Date = "2026-02-08".parse().unwrap();
    let x = b.ids.instrument("X");
    let held = e.past(day);
    let fresh = |e: &Engine| Engine::build(e.inputs().clone()).past(day);
    let check = |e: &Engine, what: &str| assert_eq!(*e.past(day), *fresh(e), "{what}: the kept holdings differ from a fresh working-out");

    // a quote now says nothing of that day
    let moved = e.apply(Change::Quote(x, Some(Quote { price: Money::new(Dec::from_int(99), held.positions[0].currency), change: None, change_pct: None, at: None, source: QuoteSource::Listing })));
    assert!(!moved.0.contains_key(&Entity::Past), "a quote moved a past day");
    check(&e, "after a quote");

    // a close after that day says nothing of it either
    let mut closes: BTreeMap<Date, Money> = e.inputs().market.closes[&x].clone();
    closes.insert("2026-02-20".parse().unwrap(), Money::new(Dec::from_int(15), closes.values().next().unwrap().currency));
    let moved = e.apply(Change::Closes(x, closes.clone()));
    assert!(!moved.0.contains_key(&Entity::Past), "a later close moved a past day");
    check(&e, "after a later close");

    // a close on or before it does, and the kept holdings are worked out again
    closes.insert("2026-02-06".parse().unwrap(), Money::new(Dec::from_int(14), closes.values().next().unwrap().currency));
    let moved = e.apply(Change::Closes(x, closes));
    assert!(moved.0.contains_key(&Entity::Past), "a close of that day did not move it");
    check(&e, "after that day's close");
    let again = e.past(day);
    assert_eq!(again.positions.iter().find(|p| p.instrument == x).unwrap().mark.as_ref().unwrap().price, Dec::from_int(14));

    // the record moves every past day
    let ledger = e.inputs().ledger.clone();
    let moved = e.apply(Change::Ledger(ledger));
    assert!(moved.0.contains_key(&Entity::Past), "a change to the record did not move it");
    check(&e, "after the record");

    // nothing held: nothing to say moved
    drop(again);
    drop(held);
    let ledger = e.inputs().ledger.clone();
    assert!(!e.apply(Change::Ledger(ledger)).0.contains_key(&Entity::Past), "a day no screen holds was said to move");
}
