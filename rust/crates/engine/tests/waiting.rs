//! What waits on the person (`Matched::waiting`, `SPEC.md` §2, What you enter):
//! the page's entry forms offer exactly these, and an entry made takes its
//! transaction off the list. Read from the engine's own cases.

mod common;

use bagholder_engine::ledger::Wanted;
use common::*;

fn case(file: &str, name_starts: &str) -> serde_json::Value {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cases").join(file);
    let cases: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    cases.into_iter().find(|c| c["name"].as_str().unwrap().starts_with(name_starts)).unwrap_or_else(|| panic!("no case {name_starts}"))
}

#[test]
fn units_moved_in_with_no_cost_wait_on_theirs_until_it_is_entered() {
    let mut b = build(&case("entries.json", "shares moved in from another broker with no cost entered"));
    let e = engine(&mut b);
    let waiting = &e.figures().matched.waiting;
    assert_eq!(waiting.len(), 1, "{waiting:?}");
    let (tx, w) = waiting.iter().next().unwrap();
    assert_eq!(w.what, Wanted::CostOfArrival);
    assert!(w.units.is_some_and(|u| u.is_positive()));
    assert_eq!(e.inputs().ledger.transactions.iter().find(|t| &t.id == tx).unwrap().kind, bagholder_core::transaction::Kind::TransferIn);

    let mut entered = build(&case("entries.json", "shares moved in from another broker take the cost and day the person entered"));
    assert!(engine(&mut entered).figures().matched.waiting.is_empty(), "a cost entered is nothing waiting");
}

#[test]
fn an_event_nothing_says_the_kind_of_waits_on_what_it_did() {
    let mut b = build(&case("events.json", "an event nothing says the kind of"));
    let e = engine(&mut b);
    let events: Vec<_> = e.figures().matched.waiting.values().filter(|w| w.what == Wanted::Event).collect();
    assert_eq!(events.len(), 1);
    let mut known = build(&case("events.json", "a spin-off with two children"));
    assert!(engine(&mut known).figures().matched.waiting.is_empty(), "an event its record explains waits on nothing");
}

#[test]
fn available_margin_waits_on_a_margin_account_whose_buying_power_is_not_read() {
    let with = |buying_power: Option<&str>| {
        let mut broker = serde_json::json!({"now": "1000", "cash": {"CAD": "-200"}});
        if let Some(b) = buying_power {
            broker["buying_power"] = serde_json::json!(b);
        }
        let case = serde_json::json!({
            "today": "2026-06-01",
            "accounts": [{"id": "M", "kind": "margin"}, {"id": "T", "kind": "cash"}],
            "instruments": [{"id": "U", "currency": "CAD", "symbol": "UUU"}],
            "transactions": [{"id": "t1", "account": "M", "day": "2026-03-02", "kind": "buy", "instrument": "U", "qty": "10", "cash": "-100"}],
            "brokers": {"M": broker},
        });
        let mut b = build(&case);
        engine(&mut b).scope(&Default::default()).portfolio.available_margin
    };
    let read = with(Some("1500.25"));
    assert_eq!(read.map(|f| f.map(|m| m.amount)), Some(Ok(bagholder_core::Dec::parse("1500.25").unwrap())));
    let unread = with(None).expect("a margin account in scope: the figure is one to state");
    assert!(unread.unwrap_err().has_word("buying-power-unread"), "it waits on the read, named");
}

#[test]
fn a_cost_the_person_entered_shows_as_entered_by_them() {
    let mut b = build(&case("entries.json", "shares moved in from another broker take the cost and day the person entered"));
    let e = engine(&mut b);
    let by_person = e.inputs().facts.adjustments.iter().all(|(_, a)| a.source == bagholder_core::SourceName::person());
    assert!(by_person, "the case's cost is the person's");
    let marked = e.figures().trades.iter().any(|t| t.flags.iter().any(|f| *f == "entered"));
    assert!(marked, "{:?}", e.figures().trades.iter().map(|t| &t.flags).collect::<Vec<_>>());
}
