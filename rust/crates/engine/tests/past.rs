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

    // the record moves every past day: a record stated again
    let restated = |e: &bagholder_engine::Engine| {
        let l = &e.inputs().ledger;
        let r = l.transactions.iter().next().unwrap().id.record;
        bagholder_engine::input::LedgerDelta { records: BTreeMap::from([(r, Some((l.records[&r].clone(), l.transactions.of_record(r).cloned().collect())))]), ..Default::default() }
    };
    let moved = e.apply(Change::Ledger(restated(&e)));
    assert!(moved.0.contains_key(&Entity::Past), "a change to the record did not move it");
    check(&e, "after the record");

    // nothing held: nothing to say moved
    drop(again);
    drop(held);
    assert!(!e.apply(Change::Ledger(restated(&e))).0.contains_key(&Entity::Past), "a day no screen holds was said to move");
}

/// A change made of rows forgets the past days from the earliest date its rows
/// state, never all of them (#527): a day before it keeps the holdings worked out
/// for it, the same object, and every day from it on equals a fresh working-out.
/// Rows are generated on many days, each with a settlement before, on and after
/// its trade date, so the earliest of the two is what decides.
#[test]
fn a_row_added_forgets_the_past_days_from_its_earliest_date_on() {
    let start: Date = "2026-02-03".parse().unwrap();
    let held_days: Vec<Date> = (0..17).map(|k| "2026-02-02".parse::<Date>().unwrap().checked_add(bagholder_core::jiff::Span::new().days(k * 5)).unwrap()).collect();
    let mut n = 0u32;
    for week in 0..10i64 {
        for settle in [-3i64, 0, 3] {
            n += 1;
            let mut b = build(&case("weekend"));
            let mut e = engine(&mut b);
            let held: Vec<_> = held_days.iter().map(|d| (*d, e.past(*d))).collect();
            let day = start.checked_add(bagholder_core::jiff::Span::new().days(week * 7)).unwrap();
            let settles = day.checked_add(bagholder_core::jiff::Span::new().days(settle)).unwrap();
            let base = e.inputs().ledger.transactions.iter().find(|t| t.instrument == Some(b.ids.instrument("X"))).unwrap().clone();
            let info = e.inputs().ledger.records[&base.id.record].clone();
            let record = bagholder_core::RecordId::parse(&format!("01890000-0000-7000-8000-{n:012}")).unwrap();
            let mut t = base.clone();
            t.id = bagholder_core::TransactionId::new(record, base.id.leg.clone());
            t.trade_date = day;
            t.settle_date = Some(settles);
            t.occurred_at = None;
            t.quantity = Some(Dec::from_int(1));
            let mut d = bagholder_engine::input::LedgerDelta::default();
            d.records.insert(record, Some((info, vec![t])));
            let first = day.min(settles);
            let moved = e.apply(Change::Ledger(d));
            let fresh = Engine::build(e.inputs().clone());
            for (h, was) in &held {
                let now = e.past(*h);
                if *h < first {
                    assert!(std::sync::Arc::ptr_eq(was, &now), "{h}, before a row of {day} settling {settles}: worked out again");
                } else {
                    assert_eq!(*now, *fresh.past(*h), "{h}, from a row of {day} settling {settles}: differs from a fresh working-out");
                }
            }
            assert_eq!(moved.0.contains_key(&Entity::Past), held.iter().any(|(h, _)| *h >= first), "a row of {day} settling {settles}: the past moved as the days it reached");
        }
    }
}
