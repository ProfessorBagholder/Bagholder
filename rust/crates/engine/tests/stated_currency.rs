//! What a broker states about an account in a currency other than CAD (an
//! account held in US dollars states its value, its deposits and what it can
//! borrow in US dollars): every figure that adds accounts together puts each in
//! CAD, the value now and what can be borrowed at the live rate, a past day's
//! value at that day's rate (`SPEC.md` portfolio-5, portfolio-34).

mod common;

use bagholder_core::{Currency, Dec, Money};
use bagholder_engine::scope::{Dates, Filters};
use bagholder_engine::Engine;
use common::*;
use serde_json::json;

fn cad(s: &str) -> Money {
    Money::new(Dec::parse(s).unwrap(), Currency::CAD)
}

fn case() -> serde_json::Value {
    json!({
        "today": "2026-03-04",
        "accounts": [{"id": "A", "kind": "margin"}, {"id": "U", "kind": "margin"}],
        "instruments": [],
        "rates": {"USD": {"2026-03-02": "1.3000", "2026-03-03": "1.4000", "2026-03-04": "1.5000"}},
        "covered": {"USD": [["2026-01-01", "2026-03-04"]]},
        "brokers": {
            "A": {"net_value": {"2026-03-02": "5000", "2026-03-03": "5100"}, "now": "9000", "buying_power": "1000"},
            "U": {"currency": "USD", "net_value": {"2026-03-02": "100", "2026-03-03": "200"}, "now": "1000", "buying_power": "500"}
        },
        "transactions": []
    })
}

#[test]
fn an_account_stated_in_us_dollars_is_added_in_cad() {
    let b = build(&case());
    let e = Engine::build(b.inputs.clone());
    let pf = e.holdings(&Filters::default());
    // now: 9000 CAD and 1000 USD at the live rate, 1.50
    assert_eq!(pf.net_value, Some(Ok(cad("10500"))), "{:?}", pf.net_value);
    // what can be borrowed: 1000 CAD and 500 USD at 1.50
    assert_eq!(pf.available_margin, Some(Ok(cad("1750"))), "{:?}", pf.available_margin);
    // a past day: each account's value then, at that day's rate (200 USD at 1.40)
    let f = Filters { dates: Dates::Range { from: None, to: Some("2026-03-03".parse().unwrap()) }, ..Filters::default() };
    let past = e.holdings(&f);
    assert_eq!(past.net_value, Some(Ok(cad("5380"))), "{:?}", past.net_value);
}

/// Without the rate the value waits on it, said as the rate missing; nothing
/// is added as though it were CAD.
#[test]
fn an_account_in_a_currency_without_its_rate_waits_on_the_rate() {
    let mut c = case();
    c["rates"] = json!({});
    c["covered"] = json!({});
    let b = build(&c);
    let e = Engine::build(b.inputs.clone());
    let pf = e.holdings(&Filters::default());
    assert!(matches!(&pf.net_value, Some(Err(g)) if g.iter().any(|x| matches!(x, bagholder_engine::gap::Gap::RateMissing { currency: Currency::USD, .. }))), "{:?}", pf.net_value);
}

/// A day whose net deposits are stated in another currency than the day before
/// forms no flow: a total restated is not money moved, at either day's rate or
/// at one. The day says why, and no return is formed over it; the days on
/// either side, stated in one currency, keep theirs.
#[test]
fn deposits_restated_in_another_currency_form_no_flow() {
    let mut c = case();
    c["rates"] = json!({"USD": {"2026-03-02": "1.3000", "2026-03-03": "1.4000", "2026-03-04": "1.5000", "2026-03-05": "1.5000"}});
    c["today"] = json!("2026-03-05");
    c["covered"] = json!({"USD": [["2026-01-01", "2026-03-05"]]});
    c["brokers"]["U"]["net_value"] = json!({"2026-03-02": "100", "2026-03-03": "200", "2026-03-04": "210", "2026-03-05": "220"});
    c["brokers"]["U"]["net_deposits"] = json!({"2026-03-02": "100", "2026-03-03": "150", "2026-03-04": "150", "2026-03-05": "160"});
    let mut b = build(&c);
    let u = b.ids.account("U");
    // the broker restates the deposits of 03-04 in CAD (150 USD at 1.50)
    let broker = b.inputs.market.brokers.get_mut(&u).unwrap();
    broker.net_deposits.insert("2026-03-04".parse().unwrap(), cad("225"));
    let broker = broker.clone();
    let eq = bagholder_engine::equity::account_equity(u, &broker, &b.inputs.facts.rates, &b.inputs.clock);
    let flow = |d: &str| eq.points.iter().find(|p| p.day.to_string() == d).unwrap().flow;
    assert_eq!(flow("2026-03-03"), Some(Dec::from_int(70)), "50 USD moved in at 1.40");
    assert_eq!(flow("2026-03-04"), None, "restated: no flow");
    assert_eq!(flow("2026-03-05"), None, "restated back: no flow");
    assert!(eq.returns.iter().all(|(d, _, _)| d.to_string() == "2026-03-03"), "{:?}", eq.returns);
    assert!(eq.gaps.iter().any(|g| matches!(g, bagholder_engine::gap::Gap::DepositsRestated { account, day } if *account == u && day.to_string() == "2026-03-04")), "{:?}", eq.gaps);
}

/// A cash-only account stated in US dollars holds nothing the figures compare
/// but its statement: when its live rate arrives, moves, or is found missing as
/// the clock passes the Bank's publication, the account is reported moved, so
/// the views that add it in CAD read it again.
#[test]
fn a_live_rate_that_moves_reports_the_accounts_stated_in_that_currency() {
    use bagholder_engine::engine::{Change, Entity};
    let mut c = case();
    c["rates"] = json!({"USD": {"2026-03-02": "1.3000", "2026-03-03": "1.4000"}});
    c["brokers"]["U"] = json!({"currency": "USD", "now": "1000", "cash": {"USD": "1000"}});
    c["covered"] = json!({});
    // 15:00 in Toronto: the day's rate is not due yet, so the live rate is the day before's
    c["now"] = json!("2026-03-04T20:00:00Z");
    let mut b = build(&c);
    let u = b.ids.account("U");
    let a = b.ids.account("A");
    let mut e = Engine::build(b.inputs.clone());
    assert_eq!(e.holdings(&Filters::default()).net_value, Some(Ok(cad("10400"))));
    // 17:00: the day's rate is due and not read, so the total waits on it
    let mut clock = b.inputs.clock.clone();
    clock.now = "2026-03-04T22:00:00Z".parse().unwrap();
    let moved = e.apply(Change::Clock(clock));
    assert!(moved.0.contains_key(&Entity::Broker(u)), "the rate became due: {moved:?}");
    assert!(matches!(e.holdings(&Filters::default()).net_value, Some(Err(_))));
    let mut rates = b.inputs.facts.rates.clone();
    rates.by_currency.get_mut(&Currency::USD).unwrap().insert("2026-03-04".parse().unwrap(), Dec::parse("1.5000").unwrap());
    let moved = e.apply(Change::Rates(rates));
    assert!(moved.0.contains_key(&Entity::Broker(u)), "the rate arrived: {moved:?}");
    assert!(!moved.0.contains_key(&Entity::Broker(a)), "a CAD account did not move: {moved:?}");
    assert_eq!(e.holdings(&Filters::default()).net_value, Some(Ok(cad("10500"))));
    // the same rates again: nothing moved
    let same = e.inputs().facts.rates.clone();
    assert!(!e.apply(Change::Rates(same)).0.contains_key(&Entity::Broker(u)));
}
