//! Debts held outside the broker (`docs/plans/debts-held-outside-wealthsimple.md`):
//! a withdrawal that repaid one is its month's repayment in the Cashflow bar,
//! and what is owed sits under the net value only while the scope covers every
//! account (brief 67, 1).

mod common;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use bagholder_core::{Currency, Dec, Money};
use bagholder_engine::input::{BrokerAccount, Debt, Debts};
use bagholder_engine::scope::Filters;
use bagholder_engine::Change;
use common::*;

fn d(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn with_a_loan() -> (Built, bagholder_engine::Engine) {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/book.json")).unwrap();
    let case: Value = serde_json::from_str(&text).unwrap();
    let mut b = build(&case);
    let mut e = engine(&mut b);
    let a = b.ids.account("A");
    e.apply(Change::Broker(a, Some(BrokerAccount { cash: BTreeMap::from([(Currency::CAD, d("1100"))]), net_value_now: Some(Money::new(d("3100"), Currency::CAD)), ..BrokerAccount::default() })));
    e.apply(Change::Debts(Debts {
        owed: vec![Debt { id: "loan-1".into(), kind: "PersonalLoan".into(), name: "Car loan".into(), institution: "Omicron Trust".into(), balance: Money::new(d("9000"), Currency::CAD), as_of: Some("2026-04-01T00:00:00Z".parse().unwrap()) }],
        repayments: BTreeMap::from([(b.tx["w1"].clone(), "loan-1".to_string())]),
            asked: vec![],
            assets: vec![], history: BTreeMap::new(),
    }));
    (b, e)
}

#[test]
fn a_repayment_is_its_months_own_part_of_the_bar() {
    let (_, e) = with_a_loan();
    let months = e.scope(&Filters::default()).cashflow.months;
    let feb = months.iter().find(|m| (m.year, m.month) == (2026, 2)).expect("a bar for the repayment's month, before the first distribution");
    assert_eq!(feb.repayments.total.clone().unwrap(), Money::new(d("200"), Currency::CAD), "shown positive");
    assert!(months.iter().filter(|m| (m.year, m.month) != (2026, 2)).all(|m| m.repayments.total.clone().unwrap().amount.is_zero()));
}

#[test]
fn a_withdrawal_no_match_named_is_no_repayment() {
    let (_, mut e) = with_a_loan();
    e.apply(Change::Debts(Debts { owed: vec![], assets: vec![], history: BTreeMap::new(), repayments: BTreeMap::new(), asked: vec![] }));
    assert!(e.scope(&Filters::default()).cashflow.months.iter().all(|m| m.repayments.total.clone().unwrap().amount.is_zero()));
}

/// A repayment is set against the income in scope whichever account the money
/// left: it stays in its month's bar under an account filter, the paying
/// account's or another's, and leaves under a holding's, which pays no debt.
#[test]
fn a_repayment_is_under_the_account_it_left_and_not_under_a_holdings() {
    // the owner, 2026-10-23: filtered to one account, Cashflow shows that account's
    // payments, not another account's repayments
    let (mut b, e) = with_a_loan();
    let feb = |e: &bagholder_engine::Engine, f: &Filters| e.scope(f).cashflow.months.iter().find(|m| (m.year, m.month) == (2026, 2)).map(|m| m.repayments.total.clone().unwrap());
    let mut only = |acc: &str| Filters { accounts: BTreeSet::from([b.ids.account(acc)]), ..Filters::default() };
    assert_eq!(feb(&e, &only("A")), Some(Money::new(d("200"), Currency::CAD)), "the account the money left");
    assert!(feb(&e, &only("B")).is_none_or(|m| m.amount.is_zero()), "another account");
    let holding = Filters { instruments: BTreeSet::from([b.ids.instrument("U")]), ..Filters::default() };
    assert!(feb(&e, &holding).is_none_or(|m| m.amount.is_zero()), "under a holding's filter");
}

#[test]
fn what_is_owed_is_stated_under_every_account_and_under_no_narrower_scope() {
    let (mut b, e) = with_a_loan();
    let (total, each) = e.scope(&Filters::default()).portfolio.owed.expect("owed under every account");
    assert_eq!(total.unwrap(), Money::new(d("9000"), Currency::CAD));
    assert_eq!(each.len(), 1);
    assert!(each[0].0.as_of.is_some(), "each debt carries when it was stated");
    for account in ["A", "C"] {
        let one = Filters { accounts: BTreeSet::from([b.ids.account(account)]), ..Filters::default() };
        assert!(e.scope(&one).portfolio.owed.is_none(), "an outside debt belongs to no account: none under {account} alone");
    }
}

#[test]
fn a_debt_in_credit_lowers_what_is_owed() {
    let (_, mut e) = with_a_loan();
    let mut d = e.inputs().facts.debts.clone();
    d.owed.push(Debt { id: "card-1".into(), kind: "PersonalLoan".into(), name: "Card".into(), institution: "Zeta Bank".into(), balance: Money::new(d_("-150"), Currency::CAD), as_of: None });
    e.apply(Change::Debts(d));
    let (total, _) = e.scope(&Filters::default()).portfolio.owed.unwrap();
    assert_eq!(total.unwrap(), Money::new(d_("8850"), Currency::CAD), "9000 owed less 150 in credit");
}

fn d_(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

#[test]
fn with_no_debt_nothing_is_owed() {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/book.json")).unwrap();
    let case: Value = serde_json::from_str(&text).unwrap();
    let mut b = build(&case);
    assert!(engine(&mut b).scope(&Filters::default()).portfolio.owed.is_none());
}

/// Each day's debts are counted as today's are: what is owed outside as last
/// stated on or before that day, and what a margin account had borrowed at that
/// day's close, worked back from the cash the broker states now through its
/// transactions; the margin is held too, so a
/// day's net worth is the accounts' stated value less what is owed outside.
#[test]
fn each_days_debts_are_the_outside_debts_and_the_margin_borrowed_at_its_close() {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/book.json")).unwrap();
    let mut case: Value = serde_json::from_str(&text).unwrap();
    // a withdrawal that takes the margin account below zero
    case["transactions"].as_array_mut().unwrap().push(serde_json::json!({"id": "w9", "account": "A", "day": "2026-03-02", "at": "2026-03-02T15:00:00Z", "kind": "withdrawal", "cash": "-6000"}));
    // and the broker states the cash it leaves, which each day's is worked back from
    case["brokers"]["A"]["cash"] = serde_json::json!({"CAD": "-2100", "USD": "-745"});
    let mut b = build(&case);
    let mut e = engine(&mut b);
    let day = |s: &str| s.parse::<bagholder_core::jiff::civil::Date>().unwrap();
    let cad = |s: &str| Money::new(d(s), Currency::CAD);
    e.apply(Change::Debts(Debts {
        owed: vec![Debt { id: "loan-1".into(), kind: "PersonalLoan".into(), name: "Car loan".into(), institution: "Omicron Trust".into(), balance: cad("9000"), as_of: None }],
        assets: vec![],
        history: BTreeMap::from([("loan-1".to_string(), BTreeMap::from([(day("2026-01-01"), cad("9500")), (day("2026-04-19"), cad("9000"))]))]),
        repayments: BTreeMap::new(),
        asked: vec![],
    }));
    let days = e.scope(&Filters::default()).portfolio.net_worth_history;
    let on = |s: &str| days.iter().find(|n| n.day == day(s)).unwrap().clone();
    assert_eq!(days.first().unwrap().day, day("2026-01-01"), "from the debt's first statement");
    assert_eq!(on("2026-01-01").debts.unwrap(), cad("9500"), "before any transaction: the outside debt alone");
    // by its close the account is below zero in each currency: 2,100 CAD (a deposit of
    // 5,000, a buy of 1,500, a sale of 600, withdrawals of 200 and 6,000) and 745 USD
    // (a buy of 900, a sale of 150, a dividend of 5), that at the day's rate
    let usd = |s: &str| bagholder_engine::fx::to_cad(&e.inputs().facts.rates, &e.inputs().clock, Money::new(d("745"), Currency::USD), day(s)).unwrap();
    let margin = cad("2100").add_to_fit(usd("2026-04-19")).unwrap();
    assert_eq!(on("2026-04-19").debts.unwrap(), cad("9000").add_to_fit(margin).unwrap(), "the outside debt and the margin borrowed");
    let before = cad("2100").add_to_fit(usd("2026-04-18")).unwrap();
    assert_eq!(on("2026-04-18").debts.unwrap(), cad("9500").add_to_fit(before).unwrap(), "the debt's earlier statement held until its next");
    assert_eq!(on("2026-04-19").net().unwrap(), cad("3000").add_to_fit(cad("-9000")).unwrap(), "the account's stated value less the outside debt");
    // under one account: what it was stated to be worth each day, and its own
    // borrowing; nothing held or owed outside, which is no account's
    let a = b.ids.account("A");
    let one = e.scope(&Filters { accounts: BTreeSet::from([a]), ..Filters::default() }).portfolio.net_worth_history;
    assert_eq!(one[0].day, day("2026-04-17"), "from the account's first stated day");
    assert_eq!(one[0].net().unwrap(), cad("3000"), "the account's stated value");
    assert_eq!(one[0].debts.clone().unwrap(), cad("2100").add_to_fit(usd("2026-04-17")).unwrap(), "its margin alone, no outside debt");
    assert!(e.scope(&Filters { instruments: BTreeSet::from([b.ids.instrument("U")]), ..Filters::default() }).portfolio.net_worth_history.is_empty(), "a holding has no net worth by day");
}

/// Cash is split by what it is for: an investment account's waits to be
/// invested and is the allocation's last slice, its shares and the holdings'
/// making the whole; a chequing account's is cash to spend, in no slice.
#[test]
fn cash_waiting_to_be_invested_is_a_slice_and_chequing_cash_is_not() {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/book.json")).unwrap();
    let mut case: Value = serde_json::from_str(&text).unwrap();
    case["accounts"].as_array_mut().unwrap().push(serde_json::json!({"id": "D", "kind": "spending"}));
    let mut b = build(&case);
    let mut e = engine(&mut b);
    let (a, dd) = (b.ids.account("A"), b.ids.account("D"));
    e.apply(Change::Broker(a, Some(BrokerAccount { cash: BTreeMap::from([(Currency::CAD, d("1000"))]), net_value_now: Some(Money::new(d("3100"), Currency::CAD)), ..BrokerAccount::default() })));
    e.apply(Change::Broker(dd, Some(BrokerAccount { cash: BTreeMap::from([(Currency::CAD, d("500"))]), ..BrokerAccount::default() })));
    let pf = e.scope(&Filters::default()).portfolio;
    assert_eq!(pf.invested_cash.clone().unwrap(), Money::new(d("1000"), Currency::CAD), "the margin account's cash");
    assert_eq!(pf.chequing_cash.clone().unwrap(), Money::new(d("500"), Currency::CAD), "the chequing account's");
    let (cash, share) = pf.allocation_cash.clone().expect("a cash slice");
    assert_eq!(cash, Money::new(d("1000"), Currency::CAD), "the invested cash alone, no chequing");
    let total: f64 = pf.allocation.iter().map(|x| x.share.clone().unwrap()).sum::<f64>() + share.unwrap();
    assert!((total - 1.0).abs() < 1e-9, "the slices make the whole: {total}");
    let holding = Filters { instruments: BTreeSet::from([b.ids.instrument("X")]), ..Filters::default() };
    assert!(e.scope(&holding).portfolio.allocation_cash.is_none(), "no cash slice under a holding's filter");
}
