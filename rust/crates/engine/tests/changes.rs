//! One change applied to a built engine gives what a fresh build on the changed
//! inputs gives, field for field, and says exactly which entities and fields
//! moved (`docs/plans/stage-2-engine.md`, the entry point). Every kind of change
//! is tried: `kind` is a `match` over `Change`, so a new kind cannot be left out.

mod common;

use std::collections::BTreeMap;

use serde_json::Value;

use bagholder_core::jiff::civil::Date;
use bagholder_core::journal::{Group, JournalEntry, JournalSubject};
use bagholder_core::{Currency, Dec, Money, SourceName};
use bagholder_engine::input::{Adjustment, AdjustmentLeg, Adjustments, BrokerAccount, Declared, DeclaredRead, Inputs, Quote, QuoteSource, Sourced};
use bagholder_engine::{Change, Engine};
use common::*;

/// The name of a change's kind. Adding a kind to `Change` fails this match
/// until the kind is named here and tried below.
fn kind(c: &Change) -> &'static str {
    match c {
        Change::Ledger(_) => "ledger",
        Change::Trades(_) => "trades",
        Change::Groups(_) => "groups",
        Change::Journal(_) => "journal",
        Change::Adjustments(_) => "adjustments",
        Change::Rates(_) => "rates",
        Change::Declared(..) => "declared",
        Change::Frequency(..) => "frequency",
        Change::Quote(..) => "quote",
        Change::Closes(..) => "closes",
        Change::Benchmark(..) => "benchmark",
        Change::Broker(..) => "broker",
        Change::Clock(_) => "clock",
    }
}

const KINDS: [&str; 13] = ["ledger", "trades", "groups", "journal", "adjustments", "rates", "declared", "frequency", "quote", "closes", "benchmark", "broker", "clock"];

/// The inputs as they are after a change: written here, apart from the engine,
/// so the test does not grade the engine by its own reading of a change.
fn changed(inputs: &Inputs, c: &Change) -> Inputs {
    let mut i = inputs.clone();
    match c.clone() {
        Change::Ledger(l) => {
            let (trades, groups, journal) = (i.ledger.trades.clone(), i.ledger.groups.clone(), i.ledger.journal.clone());
            i.ledger = l;
            i.ledger.trades = trades;
            i.ledger.groups = groups;
            i.ledger.journal = journal;
        }
        Change::Trades(t) => i.ledger.trades = t,
        Change::Groups(g) => i.ledger.groups = g,
        Change::Journal(j) => i.ledger.journal = j,
        Change::Adjustments(a) => i.facts.adjustments = a,
        Change::Rates(r) => i.facts.rates = r,
        Change::Declared(k, d) => {
            match d {
                Some(d) => i.facts.declared.insert(k, d),
                None => i.facts.declared.remove(&k),
            };
        }
        Change::Frequency(k, f) => {
            match f {
                Some(f) => i.facts.frequencies.insert(k, f),
                None => i.facts.frequencies.remove(&k),
            };
        }
        Change::Quote(k, q) => {
            match q {
                Some(q) => i.market.quotes.insert(k, q),
                None => i.market.quotes.remove(&k),
            };
        }
        Change::Closes(k, c) => {
            i.market.closes.insert(k, c);
        }
        Change::Benchmark(k, b) => {
            i.market.benchmarks.insert(k, b);
        }
        Change::Broker(k, b) => {
            match b {
                Some(b) => i.market.brokers.insert(k, b),
                None => i.market.brokers.remove(&k),
            };
        }
        Change::Clock(c) => i.clock = c,
    }
    i
}

fn d(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn day(s: &str) -> Date {
    s.parse().unwrap()
}

/// One change of every kind, each moving something on the fixture's book.
fn every_change(b: &mut Built, e: &Engine) -> Vec<Change> {
    let i = e.inputs();
    let x = b.ids.instrument("X");
    let u = b.ids.instrument("U");
    let f = b.ids.instrument("F");
    let a = b.ids.account("A");
    let mut ledger = i.ledger.clone();
    // a sale of X arrives: the match moves
    let mut sale = ledger.transactions.iter().find(|t| t.id == b.tx["t2"]).unwrap().clone();
    sale.id = bagholder_core::TransactionId::new(b.ids.record("late-sale"), bagholder_core::Leg::named("trade"));
    sale.trade_date = day("2026-03-10");
    sale.occurred_at = Some("2026-03-10T15:00:00Z".parse().unwrap());
    ledger.transactions.push(sale);
    ledger.records.insert(b.ids.record("late-sale"), bagholder_engine::input::RecordInfo { source_key: "late-sale".into(), problems: vec![] });
    let trade = e.figures().trades.iter().find_map(|t| t.trade).expect("a trade with an id");
    let second = e.figures().positions.iter().find_map(|p| p.trade).expect("a position with an id");
    let groups = vec![Group { id: b.ids.group("G"), locked: true, members: vec![trade, second] }];
    let journal = BTreeMap::from([(JournalSubject::Trade(trade), JournalEntry { thesis: "breakout".into(), grade: Some(bagholder_core::journal::Grade::A), tags: vec!["momentum".into()] })]);
    // the trade on X's round trip is gone: the round trip asks for one again
    let x_trade = e.figures().trades.iter().find(|t| t.instrument == x).and_then(|t| t.trade).expect("X's trade");
    let trades: Vec<_> = i.ledger.trades.iter().filter(|t| t.id != x_trade).cloned().collect();
    // the person states what the deposited coin cost
    let btc = b.ids.instrument("BTC");
    let adjustments = Adjustments::choose([Adjustment { applies_to: b.tx["k1"].clone(), legs: vec![AdjustmentLeg { to: Some(btc), cost: Some(Money::new(d("25000"), Currency::CAD)), ..AdjustmentLeg::default() }], source: SourceName::named("person") }]);
    let mut rates = i.facts.rates.clone();
    rates.by_currency.get_mut(&Currency::USD).unwrap().insert(day("2026-04-20"), d("1.39"));
    let declared = DeclaredRead {
        read_at: "2026-04-20T12:00:00Z".parse().unwrap(),
        source: SourceName::named("tmx"),
        items: vec![Declared { ex_date: day("2026-04-15"), record_date: None, pay_date: Some(day("2026-04-22")), amount: Money::new(d("0.12"), Currency::CAD), reinvested: None }],
    };
    let quote = Quote { price: Money::new(d("13"), Currency::CAD), change: Some(d("1")), change_pct: None, at: None, source: QuoteSource::Listing };
    // a close of the fund, which no contract is written on
    let mut closes = i.market.closes.get(&f).cloned().unwrap_or_default();
    closes.insert(day("2026-04-18"), Money::new(d("10.4"), Currency::CAD));
    let _ = u;
    let broker = BrokerAccount { cash: BTreeMap::from([(Currency::CAD, d("1100"))]), net_value_now: Some(d("3100")), as_of: Some("2026-04-20T16:00:00Z".parse().unwrap()), ..BrokerAccount::default() };
    let clock = bagholder_engine::input::Clock { today: day("2026-04-21"), now: "2026-04-21T22:00:00Z".parse().unwrap(), ..i.clock.clone() };
    let mut benchmark = i.market.benchmarks.get("SP500").cloned().unwrap_or_default();
    benchmark.insert(day("2026-04-20"), d("5210"));
    vec![
        Change::Ledger(ledger),
        Change::Trades(trades),
        Change::Groups(groups),
        Change::Journal(journal),
        Change::Adjustments(adjustments),
        Change::Rates(rates),
        Change::Declared(f, Some(declared)),
        Change::Frequency(f, Some(Sourced { value: 4, source: SourceName::named("tmx") })),
        Change::Quote(x, Some(quote)),
        Change::Closes(f, closes),
        Change::Benchmark("SP500".into(), benchmark),
        Change::Broker(a, Some(broker)),
        Change::Clock(clock),
    ]
}

#[test]
fn every_change_equals_a_fresh_build_and_says_what_moved() {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/book.json")).unwrap();
    let case: Value = serde_json::from_str(&text).unwrap();
    let mut b = build(&case);
    let base = engine(&mut b);
    let changes = every_change(&mut b, &base);
    let kinds: Vec<&str> = changes.iter().map(kind).collect();
    assert_eq!(kinds, KINDS, "one change of every kind");
    let mut failures = Vec::new();
    for c in changes {
        let k = kind(&c);
        let after = changed(base.inputs(), &c);
        let mut incremental = base.clone();
        let moved = incremental.apply(c);
        let fresh = Engine::build(after);
        let off = incremental.differences(&fresh);
        if !off.is_empty() {
            failures.push(format!("{k}: the incremental figures differ from a fresh build in {:?}", off.0));
        }
        // what moved is exactly what differs between before and after
        let expected = fresh.differences(&base);
        if moved != expected {
            failures.push(format!("{k}: moved {:?}\n   but what differs is {:?}", moved.0, expected.0));
        }
        if k != "benchmark" && expected.is_empty() {
            failures.push(format!("{k}: the change moved nothing, so it tests nothing"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
