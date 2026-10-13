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
use bagholder_engine::input::{Adjustment, AdjustmentLeg, Adjustments, BrokerAccount, Declared, DeclaredRead, Inputs, LedgerDelta, Quote, QuoteSource, Sourced};
use bagholder_engine::{Change, Engine};
use common::*;

/// The name of a change's kind. Adding a kind to `Change` fails this match
/// until the kind is named here and tried below.
fn kind(c: &Change) -> &'static str {
    match c {
        // the record, and what the person keeps on it: one change, by what it names
        Change::Ledger(d) if !d.records.is_empty() => "ledger",
        Change::Ledger(d) if !d.trades.is_empty() => "trades",
        Change::Ledger(d) if !d.groups.is_empty() => "groups",
        Change::Ledger(_) => "journal",
        Change::Adjustments(_) => "adjustments",
        Change::Rates(_) => "rates",
        Change::Declared(..) => "declared",
        Change::Frequency(..) => "frequency",
        Change::Quote(..) => "quote",
        Change::ExDividend(..) => "ex-dividend",
        Change::Closes(..) => "closes",
        Change::Benchmark(..) => "benchmark",
        Change::Broker(..) => "broker",
        Change::Clock(_) => "clock",
    }
}

const KINDS: [&str; 14] = ["ledger", "trades", "groups", "journal", "adjustments", "rates", "declared", "frequency", "quote", "ex-dividend", "closes", "benchmark", "broker", "clock"];

/// The inputs as they are after a change: written here, apart from the engine,
/// so the test does not grade the engine by its own reading of a change.
fn changed(inputs: &Inputs, c: &Change) -> Inputs {
    let mut i = inputs.clone();
    match c.clone() {
        Change::Ledger(d) => {
            // each thing the delta names, as it reads now, in place of what was held
            let l = &mut i.ledger;
            let named: std::collections::BTreeSet<_> = d.records.keys().copied().collect();
            let mut txs: Vec<_> = l.transactions.iter().filter(|t| !named.contains(&t.id.record)).cloned().collect();
            for (r, now) in &d.records {
                match now {
                    Some((info, rows)) => {
                        l.records.insert(*r, info.clone());
                        txs.extend(rows.iter().cloned());
                    }
                    None => {
                        l.records.remove(r);
                    }
                }
            }
            l.transactions = txs.into_iter().collect();
            if !named.is_empty() {
                let kept: Vec<_> = l.transfer_links.iter().filter(|(o, n)| !named.contains(&o.record) && !named.contains(&n.record)).map(|(o, n)| (o.clone(), n.clone())).collect();
                l.transfer_links = kept.into_iter().chain(d.links.iter().cloned()).collect();
            }
            for (id, t) in &d.trades {
                l.trades.remove(id);
                if let Some((rank, t)) = t {
                    l.trades.put(rank.clone(), t.clone());
                }
            }
            for (id, g) in &d.groups {
                l.groups.remove(id);
                if let Some((rank, g)) = g {
                    l.groups.put(rank.clone(), g.clone());
                }
            }
            for (k, j) in &d.journal {
                match j {
                    Some(j) => l.journal.insert(*k, j.clone()),
                    None => l.journal.remove(k),
                };
            }
        }
        Change::Adjustments(a) => {
            // every adjustment the inputs held, the named transactions' put in place, chosen again whole
            let mut all: Vec<Adjustment> = i.facts.adjustments.iter().filter(|(tx, _)| !a.contains_key(*tx)).map(|(_, x)| x.clone()).collect();
            all.extend(a.values().flatten().cloned());
            i.facts.adjustments = Adjustments::choose(all);
        }
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
        Change::ExDividend(k, d) => {
            match d {
                Some(d) => i.market.ex_dividends.insert(k, d),
                None => i.market.ex_dividends.remove(&k),
            };
        }
        Change::Closes(k, c) => {
            i.market.closes.insert(k, c);
        }
        Change::Benchmark(k, b) => {
            match b {
                Some(b) => i.market.benchmarks.insert(k, b),
                None => i.market.benchmarks.remove(&k),
            };
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
    // a sale of X arrives: the match moves
    let mut sale = i.ledger.transactions.iter().find(|t| t.id == b.tx["t2"]).unwrap().clone();
    sale.id = bagholder_core::TransactionId::new(b.ids.record("late-sale"), bagholder_core::Leg::named("trade"));
    sale.trade_date = day("2026-03-10");
    sale.occurred_at = Some("2026-03-10T15:00:00Z".parse().unwrap());
    let record = LedgerDelta { records: BTreeMap::from([(b.ids.record("late-sale"), Some((bagholder_engine::input::RecordInfo { source_key: "late-sale".into(), ..Default::default() }, vec![sale])))]), ..LedgerDelta::default() };
    let trade = e.figures().trades.iter().find_map(|t| t.trade).expect("a trade with an id");
    let second = e.figures().positions.iter().find_map(|p| p.trade).expect("a position with an id");
    let groups = LedgerDelta { groups: BTreeMap::from([(b.ids.group("G"), Some((rank(1000), Group { id: b.ids.group("G"), locked: true, members: vec![trade, second] })))]), ..LedgerDelta::default() };
    let journal = LedgerDelta { journal: BTreeMap::from([(JournalSubject::Trade(trade), Some(JournalEntry { thesis: "breakout".into(), grade: Some(bagholder_core::journal::Grade::A), tags: vec!["momentum".into()] }))]), ..LedgerDelta::default() };
    // the trade on X's round trip is gone: the round trip asks for one again
    let x_trade = e.figures().trades.iter().find(|t| t.instrument == x).and_then(|t| t.trade).expect("X's trade");
    let trades = LedgerDelta { trades: BTreeMap::from([(x_trade, None)]), ..LedgerDelta::default() };
    // the person states what the deposited coin cost
    let btc = b.ids.instrument("BTC");
    let adjustments = BTreeMap::from([(b.tx["k1"].clone(), vec![Adjustment { applies_to: b.tx["k1"].clone(), legs: vec![AdjustmentLeg { to: Some(btc), cost: Some(Money::new(d("25000"), Currency::CAD)), ..AdjustmentLeg::default() }], source: SourceName::named("person") }])]);
    let mut rates = i.facts.rates.clone();
    rates.by_currency.get_mut(&Currency::USD).unwrap().insert(day("2026-04-20"), d("1.39"));
    let declared = DeclaredRead {
        read_at: "2026-04-20T12:00:00Z".parse().unwrap(),
        source: SourceName::named("tmx"),
        items: vec![Declared { ex_date: day("2026-04-15"), record_date: None, pay_date: Some(day("2026-04-22")), amount: Money::new(d("0.12"), Currency::CAD), reinvested: None, form: bagholder_core::distribution::Form::Stated }],
        market: vec![],
    };
    let quote = Quote { price: Money::new(d("13"), Currency::CAD), change: Some(d("1")), change_pct: None, at: None, source: QuoteSource::Listing };
    // a close of the fund, which no contract is written on
    let mut closes = i.market.closes.get(&f).cloned().unwrap_or_default();
    closes.insert(day("2026-04-18"), Money::new(d("10.4"), Currency::CAD));
    let broker = BrokerAccount { cash: BTreeMap::from([(Currency::CAD, d("1100"))]), net_value_now: Some(d("3100")), as_of: Some("2026-04-20T16:00:00Z".parse().unwrap()), ..BrokerAccount::default() };
    let clock = bagholder_engine::input::Clock { today: day("2026-04-21"), now: "2026-04-21T22:00:00Z".parse().unwrap(), ..i.clock.clone() };
    let mut benchmark = i.market.benchmarks.get("SP500").cloned().unwrap_or(bagholder_engine::input::BenchmarkSeries { currency: Currency::USD, closes: BTreeMap::new(), dividends: BTreeMap::new(), splits: BTreeMap::new() });
    benchmark.closes.insert(day("2026-04-20"), d("5210"));
    vec![
        Change::Ledger(record),
        Change::Ledger(trades),
        Change::Ledger(groups),
        Change::Ledger(journal),
        Change::Adjustments(adjustments),
        Change::Rates(rates),
        Change::Declared(f, Some(declared)),
        Change::Frequency(f, Some(Sourced { value: 4, source: SourceName::named("tmx") })),
        Change::Quote(x, Some(quote)),
        // U has paid and has no declared record: its quote's ex-date is its Ex-Div
        Change::ExDividend(u, Some(day("2026-05-15"))),
        Change::Closes(f, closes),
        Change::Benchmark("SP500".into(), Some(benchmark)),
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
        if expected.is_empty() {
            failures.push(format!("{k}: the change moved nothing, so it tests nothing"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The record as a whole and each benchmark are entities of the report too, so a
/// reader of the report alone knows the accounts, the waiting facts, today and a
/// benchmark's returns moved.
#[test]
fn the_record_as_a_whole_and_each_benchmark_are_reported_when_they_move() {
    use bagholder_engine::engine::Entity;
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/book.json")).unwrap();
    let case: Value = serde_json::from_str(&text).unwrap();
    let mut b = build(&case);
    let base = engine(&mut b);
    for c in every_change(&mut b, &base) {
        let k = kind(&c);
        let moved = base.clone().apply(c);
        let book = moved.0.get(&Entity::Book);
        match k {
            "ledger" => assert!(book.is_some_and(|f| f.contains("activity")), "{k}: {:?}", moved.0),
            "clock" => assert!(book.is_some_and(|f| f.contains("today")), "{k}: {:?}", moved.0),
            "benchmark" => assert!(moved.0.contains_key(&Entity::Benchmark("SP500".into())), "{k}: {:?}", moved.0),
            "quote" | "ex-dividend" | "journal" | "groups" | "declared" | "frequency" => assert!(book.is_none(), "{k} moved the record as a whole: {:?}", moved.0),
            _ => {}
        }
    }
}
