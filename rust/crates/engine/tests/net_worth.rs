//! Net worth by day, worked out by Bagholder (`docs/plans/debts-held-outside-wealthsimple.md`):
//! a point for every day from the first anything is stated to today, each the
//! sum of what each account and each thing held or owed outside was last stated
//! to be on or before it; today's point the tiles' own figures; a day that cannot
//! be stated kept, with why. Every figure here is made up.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::jiff::civil::Date;
use bagholder_core::{Currency, Dec, Money};
use bagholder_engine::gap::Gap;
use bagholder_engine::input::{Debt, Debts};
use bagholder_engine::scope::{Filters, NetWorthDay};
use bagholder_engine::Change;
use common::*;
use serde_json::{json, Value};

fn day(s: &str) -> Date {
    s.parse().unwrap()
}

fn cad(s: &str) -> Money {
    Money::new(Dec::parse(s).unwrap(), Currency::CAD)
}

/// Two open accounts stated on different days, one in US dollars; a closed
/// account; a home stated twice and a line of credit stated once.
fn case() -> Value {
    json!({
        "name": "net worth by day",
        "today": "2026-10-20",
        "now": "2026-10-20T22:00:00Z",
        "accounts": [{"id": "A", "kind": "margin"}, {"id": "B"}, {"id": "Z", "status": "closed"}],
        "instruments": [],
        "rates": {"USD": {"2026-10-06": "1.30", "2026-10-13": "1.40", "2026-10-20": "1.50"}},
        "holidays": ["2026-10-12"],
        "brokers": {
            "A": {"cash": {"CAD": "1350"}, "net_value": {"2026-10-05": "1000", "2026-10-07": "1100", "2026-10-09": "1200", "2026-10-19": "1300"}, "now": "1350", "as_of": "2026-10-20T15:00:00Z", "buying_power": "0"},
            "B": {"currency": "USD", "cash": {"USD": "120"}, "net_value": {"2026-10-06": "100", "2026-10-13": "110"}, "now": "120", "as_of": "2026-10-20T15:00:00Z"},
            "Z": {"net_value": {"2026-10-05": "500", "2026-10-08": "400"}}
        },
        "transactions": []
    })
}

fn outside() -> Debts {
    let home = Debt { id: "home".into(), kind: "RealEstate".into(), name: "Home".into(), institution: String::new(), balance: cad("310000"), as_of: Some("2026-10-15T18:00:00Z".parse().unwrap()) };
    let loc = Debt { id: "loc".into(), kind: "LineOfCredit".into(), name: "Line of credit".into(), institution: "Bank".into(), balance: cad("5000"), as_of: Some("2026-10-07T18:00:00Z".parse().unwrap()) };
    Debts {
        owed: vec![loc],
        assets: vec![home],
        history: BTreeMap::from([
            ("home".to_string(), BTreeMap::from([(day("2026-10-06"), cad("300000")), (day("2026-10-15"), cad("310000"))])),
            ("loc".to_string(), BTreeMap::from([(day("2026-10-07"), cad("5000"))])),
        ]),
        repayments: BTreeMap::new(),
        asked: vec![],
    }
}

fn engine_of(case: &Value) -> (Built, bagholder_engine::Engine) {
    let mut b = build(case);
    let mut e = engine(&mut b);
    e.apply(Change::Debts(outside()));
    (b, e)
}

fn stated(days: &[NetWorthDay]) -> Vec<(String, String, String)> {
    days.iter().map(|n| (n.day.to_string(), n.assets.clone().unwrap().amount.to_string(), n.debts.clone().unwrap().amount.to_string())).collect()
}

fn rows(xs: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    xs.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect()
}

/// (a) Every day from the first anything is stated to today has a point, each
/// the sum of what each thing was last stated to be on or before it: an account
/// at the rate of the day it was stated for, the home held from each statement
/// to the next, the line of credit from its one statement on; (e) the closed
/// account stops after its last stated day; (b) today is the tiles' figures.
#[test]
fn every_day_is_the_sum_of_what_each_thing_was_last_stated_to_be() {
    let (_, e) = engine_of(&case());
    let days = e.scope(&Filters::default()).portfolio.net_worth_history;
    let want = rows(&[
        ("2026-10-05", "1500", "0"),          // A 1000, Z 500
        ("2026-10-06", "301630", "0"),     // and B 100 USD at 1.30, the home 300,000
        ("2026-10-07", "301730", "5000"),  // A 1100; the line of credit stated
        ("2026-10-08", "301630", "5000"),  // Z 400, its last statement
        ("2026-10-09", "301330", "5000"),  // A 1200; Z closed, counted no more
        ("2026-10-10", "301330", "5000"),
        ("2026-10-11", "301330", "5000"),
        ("2026-10-12", "301330", "5000"),
        ("2026-10-13", "301354", "5000"),  // B 110 USD at 1.40
        ("2026-10-14", "301354", "5000"),
        ("2026-10-15", "311354", "5000"),  // the home restated
        ("2026-10-16", "311354", "5000"),
        ("2026-10-17", "311354", "5000"),
        ("2026-10-18", "311354", "5000"),
        ("2026-10-19", "311454", "5000"),  // A 1300
        // today: the tiles' figures, the accounts' cash (1,350 and 120 USD at the
        // live 1.50) and the home, less the line of credit
        ("2026-10-20", "311530", "5000"),
    ]);
    assert_eq!(stated(&days), want);
}

/// (b) The last point is today's, the very figures the tiles show.
#[test]
fn todays_point_is_the_tiles() {
    let (_, e) = engine_of(&case());
    let pf = e.scope(&Filters::default()).portfolio;
    let last = pf.net_worth_history.last().unwrap();
    let now = pf.net_worth_now.clone().unwrap();
    assert_eq!(last, &now);
    assert_eq!(last.day, day("2026-10-20"));
    // the tiles' parts: the cash in the accounts, the home, the line of credit
    let assets = pf.invested_cash.clone().unwrap().add_to_fit(pf.chequing_cash.clone().unwrap()).unwrap().add_to_fit(pf.outside_assets[0].1.clone().unwrap()).unwrap();
    assert_eq!(now.assets.clone().unwrap(), assets);
    assert_eq!(now.debts.clone().unwrap(), pf.owed.clone().unwrap().0.unwrap());
    assert_eq!(now.net().unwrap(), cad("306530"));
}

/// (c) A day whose part cannot be put in CAD (the Bank's rate for the day an
/// account was stated for not stored) keeps its point, waiting on that rate, and
/// so does every day that statement is held; the days around it are stated.
#[test]
fn a_day_waiting_on_a_rate_keeps_its_point_and_says_why() {
    let mut c = case();
    c["rates"]["USD"].as_object_mut().unwrap().remove("2026-10-13");
    let (_, e) = engine_of(&c);
    let days = e.scope(&Filters::default()).portfolio.net_worth_history;
    assert_eq!(days.len(), 16, "no day vanishes");
    let gap = Gap::RateMissing { currency: Currency::USD, day: day("2026-10-13") };
    for n in &days {
        let waiting = (day("2026-10-13")..=day("2026-10-19")).contains(&n.day);
        assert_eq!(n.assets.is_err(), waiting, "{}", n.day);
        if waiting {
            assert!(n.assets.clone().unwrap_err().contains(&gap), "{}: {:?}", n.day, n.assets);
            assert!(n.net().unwrap_err().contains(&gap));
            assert_eq!(n.debts.clone().unwrap(), cad("5000"), "what is owed is still stated");
        }
    }
}

/// (d) Under an account filter: those accounts' own values, no outside thing.
#[test]
fn under_an_account_filter_nothing_outside_counts() {
    let (mut b, e) = engine_of(&case());
    let f = Filters { accounts: BTreeSet::from([b.ids.account("A"), b.ids.account("B")]), ..Filters::default() };
    let pf = e.scope(&f).portfolio;
    let got = stated(&pf.net_worth_history);
    assert_eq!(got.first().unwrap(), &rows(&[("2026-10-05", "1000", "0")])[0]);
    assert_eq!(got.iter().find(|r| r.0 == "2026-10-15").unwrap(), &rows(&[("2026-10-15", "1354", "0")])[0], "no home, no line of credit");
    assert_eq!(got.last().unwrap(), &rows(&[("2026-10-20", "1530", "0")])[0], "today: the two accounts' cash");
    assert_eq!(pf.net_worth_history.last(), pf.net_worth_now.as_ref());
}

/// (e) A closed account stops after its last stated day; an open one's last
/// statement is held to today.
#[test]
fn a_closed_account_stops_after_its_last_stated_day() {
    let (mut b, e) = engine_of(&case());
    let mut only = |a: &str| e.scope(&Filters { accounts: BTreeSet::from([b.ids.account(a)]), ..Filters::default() }).portfolio.net_worth_history;
    let z = only("Z");
    assert_eq!(stated(&z[..4]), rows(&[("2026-10-05", "500", "0"), ("2026-10-06", "500", "0"), ("2026-10-07", "500", "0"), ("2026-10-08", "400", "0")]));
    assert!(z[4..z.len() - 1].iter().all(|n| n.assets.clone().unwrap().amount.is_zero()), "nothing after its last day");
    let a = only("A");
    assert_eq!(stated(&a[a.len() - 2..a.len() - 1]), rows(&[("2026-10-19", "1300", "0")]));
}

/// Under a range, the points in it; one ending before today, its last day's
/// figures are the tiles'.
#[test]
fn a_past_range_ends_at_its_last_day() {
    let (_, e) = engine_of(&case());
    let f = Filters { dates: bagholder_engine::scope::Dates::Range { from: Some(day("2026-10-06")), to: Some(day("2026-10-09")) }, ..Filters::default() };
    let pf = e.scope(&f).portfolio;
    assert_eq!(pf.net_worth_history.iter().map(|n| n.day.to_string()).collect::<Vec<_>>(), ["2026-10-06", "2026-10-07", "2026-10-08", "2026-10-09"]);
    let _ = pf;
}
