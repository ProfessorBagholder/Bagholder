//! The facts the person enters (`SPEC.md` §2, "What you enter"): each its own
//! record, derived into an adjustment on the transaction it explains, and
//! refused where the book cannot stand behind it.

mod common;

use bagholder_book::person::Entry;
use bagholder_book::BookError;
use bagholder_core::{Currency, Dec, Money, SourceName};
use common::*;
use serde_json::json;

fn arrival(f: &Fixture) -> (bagholder_core::TransactionId, bagholder_core::InstrumentId) {
    f.account(&["tfsa-1"]);
    let r = f.store(&Spelled::v(1), "in", &legs(vec![json!({"leg": "trade", "account": "tfsa-1", "kind": "transfer-in", "instrument": share("CA0000000001", "XYZ"), "quantity": "100", "at": "2026-02-01T15:00:00Z", "date": "2026-02-01"})]));
    let t = f.book.transactions_of(r.record).unwrap().remove(0);
    (t.id, t.instrument.unwrap())
}

#[test]
fn the_cost_of_an_arrival_is_an_adjustment_of_the_person_s_on_it() {
    let f = Fixture::new();
    let (t, i) = arrival(&f);
    f.book.enter(&Entry::CostOfArrival { arrival: t.clone(), instrument: i, cost: cad("1234.56"), acquired: "2020-05-01".parse().unwrap() }, t0()).unwrap();
    let all = f.book.adjustments().unwrap();
    let a = all.iter().find(|a| a.applies_to == t).expect("an adjustment on the arrival");
    assert_eq!(a.source, SourceName::person());
    assert_eq!(a.legs[0].to, Some(i));
    assert_eq!(a.legs[0].cost, Some(cad("1234.56")));
    assert_eq!(a.legs[0].acquired, Some("2020-05-01".parse().unwrap()));
}

#[test]
fn a_return_of_capital_is_cash_per_unit_on_its_distribution() {
    let f = Fixture::new();
    let (t, i) = arrival(&f);
    f.book.enter(&Entry::ReturnOfCapital { distribution: t.clone(), instrument: i, per_unit: cad("0.25") }, t0()).unwrap();
    let a = f.book.adjustments().unwrap().into_iter().find(|a| a.applies_to == t).unwrap();
    assert_eq!((a.legs[0].from, a.legs[0].to, a.legs[0].cash_per_unit), (Some(i), Some(i), Some(cad("0.25"))));
}

#[test]
fn an_entry_the_book_cannot_stand_behind_is_refused_and_named() {
    let f = Fixture::new();
    let (t, i) = arrival(&f);
    let usd = Money::new(d("10"), Currency::USD);
    let refused = |e: Result<_, BookError>| matches!(e, Err(BookError::Refused(_)));
    // a cost in another currency than the instrument's
    assert!(refused(f.book.enter(&Entry::CostOfArrival { arrival: t.clone(), instrument: i, cost: usd, acquired: "2020-05-01".parse().unwrap() }, t0())));
    // a share of cost that is not a part of the whole
    assert!(refused(f.book.enter(&Entry::SpinOff { event: t.clone(), parent: i, children: vec![(i, d("1.5"))] }, t0())));
    // shares that add up past the whole
    assert!(refused(f.book.enter(&Entry::SpinOff { event: t.clone(), parent: i, children: vec![(i, d("0.6")), (i, d("0.6"))] }, t0())));
    // capital returned that is not an amount
    assert!(refused(f.book.enter(&Entry::ReturnOfCapital { distribution: t, instrument: i, per_unit: cad("0") }, t0())));
    assert!(f.book.adjustments().unwrap().is_empty());
    let _ = Dec::ONE;
}
