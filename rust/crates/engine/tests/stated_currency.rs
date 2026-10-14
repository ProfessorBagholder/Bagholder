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
