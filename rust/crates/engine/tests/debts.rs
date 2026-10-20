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
            assets: vec![], history: vec![], owed_history: vec![],
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
    e.apply(Change::Debts(Debts { owed: vec![], assets: vec![], history: vec![], owed_history: vec![], repayments: BTreeMap::new(), asked: vec![] }));
    assert!(e.scope(&Filters::default()).cashflow.months.iter().all(|m| m.repayments.total.clone().unwrap().amount.is_zero()));
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

/// Each day's debts are counted as today's are: what is owed outside as stated
/// that day, and what a margin account had borrowed at that day's close, worked
/// out from its transactions; net worth stays as stated.
#[test]
fn each_days_debts_are_the_outside_debts_and_the_margin_borrowed_at_its_close() {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/book.json")).unwrap();
    let mut case: Value = serde_json::from_str(&text).unwrap();
    // a withdrawal that takes the margin account below zero
    case["transactions"].as_array_mut().unwrap().push(serde_json::json!({"id": "w9", "account": "A", "day": "2026-03-02", "at": "2026-03-02T15:00:00Z", "kind": "withdrawal", "cash": "-6000"}));
    let mut b = build(&case);
    let mut e = engine(&mut b);
    let day = |s: &str| s.parse::<bagholder_core::jiff::civil::Date>().unwrap();
    let cad = |s: &str| Money::new(d(s), Currency::CAD);
    e.apply(Change::Debts(Debts {
        owed: vec![Debt { id: "loan-1".into(), kind: "PersonalLoan".into(), name: "Car loan".into(), institution: "Omicron Trust".into(), balance: cad("9000"), as_of: None }],
        assets: vec![],
        history: vec![(day("2026-01-01"), cad("1000")), (day("2026-04-19"), cad("2000"))],
        owed_history: vec![(day("2026-01-01"), cad("9500")), (day("2026-04-19"), cad("9000"))],
        repayments: BTreeMap::new(),
        asked: vec![],
    }));
    let days = e.scope(&Filters::default()).portfolio.net_worth_history;
    assert_eq!(days.iter().map(|n| n.net).collect::<Vec<_>>(), [cad("1000"), cad("2000")], "net worth as stated");
    assert_eq!(days[0].debts, Some(cad("9500")), "before any transaction: the outside debts alone");
    // by its close the account is below zero in each currency: 2,100 CAD (a deposit of
    // 5,000, a buy of 1,500, a sale of 600, withdrawals of 200 and 6,000) and 745 USD
    // (a buy of 900, a sale of 150, a dividend of 5), that at the day's rate
    let usd = bagholder_engine::fx::to_cad(&e.inputs().facts.rates, &e.inputs().clock, Money::new(d("745"), Currency::USD), day("2026-04-19")).unwrap();
    assert_eq!(days[1].debts, Some(cad("11100").add_to_fit(usd).unwrap()), "the outside debts and the margin borrowed");
    let a = b.ids.account("A");
    assert!(e.scope(&Filters { accounts: BTreeSet::from([a]), ..Filters::default() }).portfolio.net_worth_history.is_empty(), "under every account only");
}
