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

#[test]
fn a_trade_entered_by_hand_is_the_transaction_it_states() {
    use bagholder_book::person::{Side, Traded};
    let f = Fixture::new();
    let (_, held) = arrival(&f);
    let account = f.book.accounts().unwrap()[0].id;
    // a sale of what the book holds: the same instrument, the units going out
    let sold = f
        .book
        .enter(&Entry::Trade { account, instrument: Traded::Held(held), day: "2026-03-02".parse().unwrap(), side: Side::Sell, quantity: d("40"), price: cad("12.5"), fee: Some(cad("4.95")) }, t0())
        .unwrap();
    let t = f.book.transactions_of(sold.record).unwrap().remove(0);
    assert_eq!((t.kind, t.instrument, t.quantity, t.price, t.fee, t.cash), (bagholder_core::transaction::Kind::Sell, Some(held), Some(d("-40")), Some(cad("12.5")), Some(cad("4.95")), None), "the price and the fee as entered, no cash worked out");
    assert_eq!(t.trade_date, "2026-03-02".parse().unwrap());
    // a purchase of a symbol the book has not met: an instrument named by it, in the account's connection
    let bought = f
        .book
        .enter(&Entry::Trade { account, instrument: Traded::Named { symbol: " qnc ".into(), currency: Currency::USD, contract: None }, day: "2026-03-03".parse().unwrap(), side: Side::Buy, quantity: d("10"), price: Money::new(d("1.75"), Currency::USD), fee: None }, t0())
        .unwrap();
    let t = f.book.transactions_of(bought.record).unwrap().remove(0);
    let i = t.instrument.unwrap();
    assert_ne!(i, held);
    assert_eq!(f.book.instrument(i).unwrap().currency, Currency::USD);
    assert_eq!(f.book.names(i).unwrap().last().unwrap().symbol, "QNC", "named as entered");
    assert_eq!(t.quantity, Some(d("10")));
    assert_eq!(f.book.record(bought.record).unwrap().source, SourceName::person());
}

#[test]
fn a_trade_the_book_cannot_stand_behind_is_refused_and_named() {
    use bagholder_book::person::{Side, Traded};
    let f = Fixture::new();
    let (_, held) = arrival(&f);
    let account = f.book.accounts().unwrap()[0].id;
    let trade = |quantity: &str, price: Money, fee: Option<Money>, instrument: Traded| Entry::Trade { account, instrument, day: "2026-03-02".parse().unwrap(), side: Side::Buy, quantity: d(quantity), price, fee };
    let refused = |e: &Entry| matches!(f.book.enter(e, t0()), Err(BookError::Refused(_)));
    assert!(refused(&trade("0", cad("1"), None, Traded::Held(held))), "no units");
    assert!(refused(&trade("-5", cad("1"), None, Traded::Held(held))), "units are counted up, the side says which way");
    assert!(refused(&trade("5", Money::new(d("1"), Currency::USD), None, Traded::Held(held))), "a price in another currency than the instrument's");
    assert!(refused(&trade("5", cad("-1"), None, Traded::Held(held))), "a negative price");
    assert!(refused(&trade("5", cad("1"), Some(cad("-1")), Traded::Held(held))), "a negative fee");
    assert!(refused(&trade("5", cad("1"), None, Traded::Named { symbol: "  ".into(), currency: Currency::CAD, contract: None })), "no symbol");
}

#[test]
fn a_contract_entered_by_hand_is_an_option_on_its_underlying_with_its_size_unstated() {
    use bagholder_book::person::{Contract, Side, Traded, Underlying};
    let f = Fixture::new();
    let (_, held) = arrival(&f);
    let account = f.book.accounts().unwrap()[0].id;
    let contract = Contract { underlying: Underlying::Held(held), expiry: "2027-01-15".parse().unwrap(), strike: d("12"), right: bagholder_core::instrument::OptionRight::Call };
    let stored = f
        .book
        .enter(&Entry::Trade { account, instrument: Traded::Named { symbol: "XYZ 15JAN27 12.00 CALL".into(), currency: Currency::CAD, contract: Some(contract) }, day: "2026-03-02".parse().unwrap(), side: Side::Buy, quantity: d("2"), price: cad("1.10"), fee: None }, t0())
        .unwrap();
    let t = f.book.transactions_of(stored.record).unwrap().remove(0);
    let i = t.instrument.unwrap();
    assert_eq!(f.book.instrument(i).unwrap().kind, bagholder_core::instrument::InstrumentKind::OptionContract);
    let terms = f.book.option_terms(i).unwrap().expect("its terms");
    assert_eq!((terms.underlying, terms.strike, terms.multiplier), (held, d("12"), None), "on the instrument the book holds, its size not assumed");
}
