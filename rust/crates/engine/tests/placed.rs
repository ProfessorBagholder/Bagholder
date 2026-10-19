//! A round trip added inside one of a family's flat stretches is placed there
//! (`docs/plans/matching-from-a-flat-point.md`), and any other row added before
//! what a family matched is walked on from the cut before it
//! (`docs/plans/continuing-from-a-flat-stretch.md`); either equals the family
//! matched whole.
//! Each step is labelled by the test with the path it expects for the family it
//! names (placed, or matched whole and why), never by asking the guard; the
//! engine's record of the path must match the label. After every step the engine
//! equals a fresh build in every figure, in its match and identity, in what moved,
//! and in the cashflow a side filter reads through the round trip entitled to
//! each payment. Under the `oracles` feature every placed step is also checked
//! against matching its family whole, kept state and flat stretches included.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use bagholder_core::transaction::Transaction;
use bagholder_core::{RecordId, TransactionId};
use bagholder_engine::engine::Path as Took;
use bagholder_engine::families::Node;
use bagholder_engine::input::{Inputs, LedgerDelta, RecordInfo};
use bagholder_engine::ledger::{match_lots, Direction};
use bagholder_engine::scope::Filters;
use bagholder_engine::{Change, Engine};
use common::*;

/// What a step expects of the family of the instrument it names.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Path {
    Placed,
    /// Walked on from the cut before its rows.
    FromCut,
    /// After everything the family matched.
    Continued,
    /// Matched whole, and the reason the test knows it must be.
    Whole(&'static str),
}

struct Step {
    records: Vec<&'static str>,
    on: &'static str,
    path: Path,
}

/// The book: P's round trips across two accounts with flat stretches between
/// them and its last one open (four units), payments in its longest stretch, two
/// of them for a declared distribution; another family V; W, whose contract was
/// opened and closed, flat from January to November. Every row a step adds is
/// here too, each its own record unless it says. Amounts vary by seed.
fn universe(seed: u64) -> Value {
    let mut n = seed;
    let mut cash = move || {
        n = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (10 + (n >> 33) % 900).to_string()
    };
    let row = |id: &str, account: &str, at: &str, kind: &str, instrument: &str, qty: &str, cash: String| {
        let day = &at[..10];
        let mut r = json!({"id": id, "account": account, "day": day, "kind": kind, "instrument": instrument, "cash": cash});
        if at.len() > 10 {
            r["at"] = json!(at);
        }
        if !qty.is_empty() {
            r["qty"] = json!(qty);
        }
        r
    };
    let buy = |id: &str, account: &str, at: &str, i: &str, q: &str, c: String| row(id, account, at, "buy", i, q, format!("-{c}"));
    let sell = |id: &str, account: &str, at: &str, i: &str, q: &str, c: String| row(id, account, at, "sell", i, &format!("-{q}"), c);
    let paid = |id: &str, at: &str, c: String| row(id, "A", at, "dividend", "P", "", c);
    let mut tx = vec![
        // P: A's round trip, B's, A's again, then A's open one
        buy("p1", "A", "2026-01-05T15:00:00Z", "P", "10", cash()),
        sell("p2", "A", "2026-01-20T15:00:00Z", "P", "10", cash()),
        buy("pb1", "B", "2026-02-10T15:00:00Z", "P", "5", cash()),
        sell("pb2", "B", "2026-02-12T15:00:00Z", "P", "5", cash()),
        buy("p3", "A", "2026-03-02T15:00:00Z", "P", "4", cash()),
        sell("p4", "A", "2026-03-20T15:00:00Z", "P", "4", cash()),
        buy("p5", "A", "2026-10-01T15:00:00Z", "P", "4", cash()),
        // payments the walk passes over, in the stretch from March to October
        paid("d1", "2026-06-10T20:00:00Z", cash()),
        paid("d2", "2026-06-20T20:00:00Z", cash()),
        paid("d3", "2026-06-30T20:00:00Z", cash()),
        paid("d4", "2026-07-10T20:00:00Z", cash()),
        paid("d5", "2026-07-20T20:00:00Z", cash()),
        // paid for the declared distributions: one gone ex inside x9, one before it
        paid("dp1", "2026-09-15T20:00:00Z", cash()),
        paid("dp2", "2026-09-25T20:00:00Z", cash()),
        buy("v1", "A", "2026-04-14T15:00:00Z", "V", "3", cash()),
        // W and its contract
        buy("w1", "A", "2026-01-06T15:00:00Z", "W", "100", cash()),
        json!({"id": "ow1", "account": "A", "day": "2026-01-07", "at": "2026-01-07T15:00:00Z", "kind": "sell", "effect": "open", "instrument": "OW", "qty": "-1", "cash": cash()}),
        json!({"id": "ow2", "account": "A", "day": "2026-01-14", "at": "2026-01-14T15:00:00Z", "kind": "buy", "effect": "close", "instrument": "OW", "qty": "1", "cash": format!("-{}", cash())}),
        sell("w2", "A", "2026-01-15T15:00:00Z", "W", "100", cash()),
        buy("w3", "A", "2026-11-02T15:00:00Z", "W", "10", cash()),
        // K, a coin: a round trip, then one open
        buy("k1", "C", "2026-01-05T15:00:00Z", "K", "1.000000", cash()),
        sell("k2", "C", "2026-01-10T15:00:00Z", "K", "1.000000", cash()),
        buy("k3", "C", "2026-06-01T15:00:00Z", "K", "0.500000", cash()),
        // R: one round trip, then only its contract, opened and closed
        buy("r1", "A", "2026-01-05T15:00:00Z", "R", "10", cash()),
        sell("r2", "A", "2026-01-10T15:00:00Z", "R", "10", cash()),
        json!({"id": "or1", "account": "A", "day": "2026-05-01", "at": "2026-05-01T15:00:00Z", "kind": "sell", "effect": "open", "instrument": "OR", "qty": "-1", "cash": cash()}),
        json!({"id": "or2", "account": "A", "day": "2026-05-10", "at": "2026-05-10T15:00:00Z", "kind": "buy", "effect": "close", "instrument": "OR", "qty": "1", "cash": format!("-{}", cash())}),
        // the steps' rows: placed
        buy("x1b", "A", "2026-04-01T15:00:00Z", "P", "4", cash()),
        sell("x1s", "A", "2026-04-03T15:00:00Z", "P", "4", cash()),
        buy("x2b", "A", "2026-04-20T15:00:00Z", "P", "2", cash()),
        sell("x2s", "A", "2026-04-20T16:00:00Z", "P", "2", cash()),
        buy("x3b", "A", "2026-04-15T15:00:00Z", "P", "1", cash()),
        sell("x3s", "A", "2026-04-15T16:00:00Z", "P", "1", cash()),
        buy("v2", "A", "2026-04-15T15:30:00Z", "V", "1", cash()),
        buy("x4b", "A", "2026-06-12T15:00:00Z", "P", "3", cash()),
        sell("x4s", "A", "2026-06-14T15:00:00Z", "P", "3", cash()),
        buy("x5b", "A", "2026-06-20T15:00:00Z", "P", "3", cash()),
        sell("x5s", "A", "2026-06-22T15:00:00Z", "P", "3", cash()),
        buy("x6b", "A", "2026-06-28T15:00:00Z", "P", "3", cash()),
        sell("x6s", "A", "2026-07-02T15:00:00Z", "P", "3", cash()),
        buy("x7b", "A", "2026-07-08T15:00:00Z", "P", "3", cash()),
        sell("x7s", "A", "2026-07-10T15:00:00Z", "P", "3", cash()),
        buy("x8b", "A", "2026-07-15T15:00:00Z", "P", "3", cash()),
        sell("x8s", "A", "2026-07-18T15:00:00Z", "P", "3", cash()),
        buy("x9b", "A", "2026-08-05T15:00:00Z", "P", "3", cash()),
        sell("x9s", "A", "2026-08-20T15:00:00Z", "P", "3", cash()),
        buy("xwb", "A", "2026-05-05T15:00:00Z", "W", "5", cash()),
        sell("xws", "A", "2026-05-07T15:00:00Z", "W", "5", cash()),
        buy("xrb", "A", "2026-02-01T15:00:00Z", "R", "2", cash()),
        sell("xrs", "A", "2026-02-03T15:00:00Z", "R", "2", cash()),
        // matched whole
        buy("y1b", "A", "2026-03-20T16:00:00Z", "P", "1", cash()),
        sell("y1s", "A", "2026-03-20T17:00:00Z", "P", "1", cash()),
        buy("y2b", "A", "2026-02-10T12:00:00Z", "P", "1", cash()),
        sell("y2s", "A", "2026-02-10T13:00:00Z", "P", "1", cash()),
        buy("y3b", "A", "2026-01-25T15:00:00Z", "P", "1", cash()),
        sell("y3s", "A", "2026-02-15T15:00:00Z", "P", "1", cash()),
        json!({"id": "y4b", "account": "A", "day": "2026-07-01", "at": "2026-07-01T15:00:00Z", "kind": "buy", "effect": "open", "instrument": "OW2", "qty": "1", "cash": format!("-{}", cash())}),
        json!({"id": "y4s", "account": "A", "day": "2026-07-02", "at": "2026-07-02T15:00:00Z", "kind": "sell", "effect": "close", "instrument": "OW2", "qty": "-1", "cash": cash()}),
        row("y5i", "A", "2026-06-05T15:00:00Z", "transfer-in", "W", "2", String::new()),
        row("y5o", "A", "2026-06-06T15:00:00Z", "transfer-out", "W", "-2", String::new()),
        json!({"id": "y6p", "record": "y6", "account": "A", "day": "2026-08-27", "kind": "buy", "instrument": "P", "qty": "1", "cash": format!("-{}", cash())}),
        json!({"id": "y6v", "record": "y6", "account": "A", "day": "2026-08-27", "kind": "sell", "instrument": "V", "qty": "-1", "cash": cash()}),
        json!({"id": "y6s", "record": "y6", "account": "A", "day": "2026-08-27", "kind": "sell", "instrument": "P", "qty": "-1", "cash": cash()}),
        sell("y7", "A", "2026-08-25T15:00:00Z", "P", "1", cash()),
        json!({"id": "wu", "account": "A", "day": "2026-11-05", "at": "2026-11-05T15:00:00Z", "kind": "buy", "instrument": "W", "qty": "1", "cash": "-10", "problems": ["leg-unstated"]}),
        buy("y8b", "A", "2026-06-01T15:00:00Z", "W", "1", cash()),
        sell("y8s", "A", "2026-06-03T15:00:00Z", "W", "1", cash()),
        buy("y9", "A", "2026-08-28T15:00:00Z", "P", "1", cash()),
        buy("y10b", "C", "2026-03-01T15:00:00Z", "K", "0.200000", cash()),
        sell("y10s", "C", "2026-03-03T15:00:00Z", "K", "0.200000", cash()),
        // Q: a round trip in B, then after it a buy, a split, a move between the
        // accounts linked on both sides, a return of capital, and a put written
        // and assigned: rows a walk from the cut before them meets
        buy("q1", "B", "2026-01-05T15:00:00Z", "Q", "10", cash()),
        sell("q2", "B", "2026-01-10T15:00:00Z", "Q", "10", cash()),
        buy("q3", "B", "2026-04-01T15:00:00Z", "Q", "4", cash()),
        json!({"id": "qe", "account": "B", "day": "2026-05-01", "at": "2026-05-01T11:00:00Z", "kind": "corporate-event", "instrument": "Q"}),
        row("qto", "B", "2026-06-01T15:00:00Z", "transfer-out", "Q", "-2", String::new()),
        row("qti", "A", "2026-06-01T15:01:00Z", "transfer-in", "Q", "2", String::new()),
        row("qr", "A", "2026-07-01T20:00:00Z", "dividend", "Q", "", "1".into()),
        json!({"id": "oq1", "account": "B", "day": "2026-07-06", "at": "2026-07-06T15:00:00Z", "kind": "sell", "effect": "open", "instrument": "OQ", "qty": "-1", "cash": cash()}),
        json!({"id": "oq2", "account": "B", "day": "2026-08-03", "at": "2026-08-03T21:30:00Z", "kind": "option-assignment", "instrument": "OQ", "qty": "1", "cash": "-1000"}),
        // after R's round trip placed: a round trip continued after its last row,
        // then a buy before that one which stays open, walked on from a cut that
        // must hold the placed trip
        buy("rq1", "A", "2026-05-20T15:00:00Z", "R", "2", cash()),
        sell("rq2", "A", "2026-05-25T15:00:00Z", "R", "2", cash()),
        buy("rq3", "A", "2026-05-22T15:00:00Z", "R", "1", cash()),
        // T2: a round trip, then a row continued on the day it went flat, a sale
        // after it, and a buy before that sale which stays open: the cut of that
        // first day no longer holds the day
        buy("tt1", "C", "2026-01-05T15:00:00Z", "T2", "3", cash()),
        sell("tt2", "C", "2026-01-10T15:00:00Z", "T2", "3", cash()),
        buy("tc1", "C", "2026-01-10T16:00:00Z", "T2", "1", cash()),
        sell("tc2", "C", "2026-01-20T15:00:00Z", "T2", "1", cash()),
        buy("tc3", "C", "2026-01-15T15:00:00Z", "T2", "1", cash()),
        // the steps on Q: a buy before them that stays open, then an expiry and an event added
        buy("zq1", "B", "2026-02-02T15:00:00Z", "Q", "1", cash()),
        json!({"id": "zq2", "account": "B", "day": "2026-02-03", "at": "2026-02-03T21:00:00Z", "kind": "option-expiry", "instrument": "OQ2", "qty": "1"}),
        json!({"id": "zq3", "account": "B", "day": "2026-02-04", "at": "2026-02-04T11:00:00Z", "kind": "corporate-event", "instrument": "Q"}),
    ];
    for t in &mut tx {
        if t["cash"] == json!("") {
            t.as_object_mut().unwrap().remove("cash");
        }
    }
    json!({
        "today": "2026-12-31",
        "accounts": [{"id": "A", "kind": "margin"}, {"id": "B"}, {"id": "C"}],
        "instruments": [
            {"id": "P"}, {"id": "V"}, {"id": "W"}, {"id": "R"}, {"id": "K", "kind": "crypto"},
            {"id": "OR", "kind": "option", "underlying": "R", "expiry": "2026-06-19", "strike": "20", "right": "call", "multiplier": "100"},
            {"id": "Q"}, {"id": "T2"},
            {"id": "OQ", "kind": "option", "underlying": "Q", "expiry": "2026-08-21", "strike": "10", "right": "put", "multiplier": "100"},
            {"id": "OQ2", "kind": "option", "underlying": "Q", "expiry": "2026-02-03", "strike": "12", "right": "call", "multiplier": "100"},
            {"id": "OW", "kind": "option", "underlying": "W", "expiry": "2026-01-16", "strike": "20", "right": "call", "multiplier": "100"},
            {"id": "OW2", "kind": "option", "underlying": "W", "expiry": "2026-12-18", "strike": "25", "right": "call", "multiplier": "100"}
        ],
        "declared": {"P": [
            {"ex": "2026-08-12", "pay": "2026-09-15", "amount": "0.50"},
            {"ex": "2026-08-01", "pay": "2026-09-25", "amount": "0.50"}
        ]},
        "transactions": tx,
        "transfer_links": [{"out": "qto", "in": "qti"}],
        "adjustments": [
            {"applies_to": "qe", "legs": [{"from": "Q", "to": "Q", "units_per_unit": "2"}]},
            {"applies_to": "zq3", "legs": [{"from": "Q", "to": "Q", "units_per_unit": "3"}]},
            {"applies_to": "qr", "legs": [{"from": "Q", "to": "Q", "cash_per_unit": "0.10"}]}
        ],
    })
}

const BASE: &[&str] = &["p1", "p2", "pb1", "pb2", "p3", "p4", "p5", "d1", "d2", "d3", "d4", "d5", "dp1", "dp2", "v1", "w1", "ow1", "ow2", "w2", "w3", "r1", "r2", "or1", "or2", "k1", "k2", "k3", "q1", "q2", "q3", "qe", "qto", "qti", "qr", "oq1", "oq2", "tt1", "tt2"];

fn steps() -> Vec<Step> {
    use Path::*;
    let st = |records: Vec<&'static str>, on, path| Step { records, on, path };
    vec![
        // over days, buying the units the holding holds at the end
        st(vec!["x1b", "x1s"], "P", Placed),
        // the same-day round trip (brief 60, 2): alone, and beside another family's row that day
        st(vec!["x2b", "x2s"], "P", Placed),
        st(vec!["x3b", "x3s", "v2"], "P", Placed),
        // round trips around payments the walk passes over (brief 60, 1): a payment
        // before the buy, on its day, between, on the sale's day, after it
        st(vec!["x4b", "x4s"], "P", Placed),
        st(vec!["x5b", "x5s"], "P", Placed),
        st(vec!["x6b", "x6s"], "P", Placed),
        st(vec!["x7b", "x7s"], "P", Placed),
        st(vec!["x8b", "x8s"], "P", Placed),
        // open over one declared distribution's ex-date, and opened after the other's
        st(vec!["x9b", "x9s"], "P", Placed),
        // in a family with a share and its contract, flat on the day
        st(vec!["xwb", "xws"], "W", Placed),
        // the last price R's fills state is the placed sale's: no fill of R after it
        st(vec!["xrb", "xrs"], "R", Placed),
        st(vec!["rq1"], "R", Continued),
        st(vec!["rq2"], "R", Continued),
        st(vec!["rq3"], "R", FromCut),
        st(vec!["tc1"], "T2", Continued),
        st(vec!["tc2"], "T2", Continued),
        st(vec!["tc3"], "T2", Whole("no clean cut before it")),
        // not placed, so walked on from the cut before them: on the day a stretch
        // starts or ends, over two stretches, a contract, a transfer, a record with a
        // leg in another family, a sale of more than is held, a coin
        st(vec!["y1b", "y1s"], "P", FromCut),
        st(vec!["y2b", "y2s"], "P", FromCut),
        st(vec!["y3b", "y3s"], "P", FromCut),
        st(vec!["y4b", "y4s"], "W", FromCut),
        st(vec!["y5i", "y5o"], "W", FromCut),
        st(vec!["y6p", "y6v", "y6s"], "P", FromCut),
        st(vec!["y7"], "P", FromCut),
        st(vec!["wu"], "W", Whole("a record whose legs are not stated")),
        // the cuts carry the taint
        st(vec!["y8b", "y8s"], "W", FromCut),
        // a buy that stays open, joining every later round trip
        st(vec!["y9"], "P", FromCut),
        st(vec!["y10b", "y10s"], "K", FromCut),
        // a buy that stays open before a split, a linked move, a return of capital
        // and an assignment, all walked on from the cut
        st(vec!["zq1"], "Q", FromCut),
        st(vec!["zq2"], "Q", Whole("an expiry row decides how an earlier lot ended")),
        st(vec!["zq3"], "Q", Whole("a corporate event")),
    ]
}

/// The cashflow a side filter answers through each payment's round trip.
fn by_side(e: &Engine) -> bagholder_engine::scope::Cashflow {
    let f = Filters { sides: BTreeSet::from([Direction::Long]), ..Filters::default() };
    let p = e.portfolio(&f);
    e.cashflow(&f, &p)
}

#[test]
fn a_round_trip_placed_in_a_flat_stretch_equals_a_whole_match() {
    let (mut placed_steps, mut cut_steps) = (0, 0);
    for seed in 1..=10u64 {
        let case = universe(seed);
        let mut b = build(&case);
        let named: BTreeMap<&str, bagholder_core::InstrumentId> = ["P", "V", "W", "R", "K", "Q", "T2"].into_iter().map(|i| (i, b.ids.instrument(i))).collect();
        let full = b.inputs.clone();
        let whole: BTreeMap<RecordId, (RecordInfo, Vec<Transaction>)> = full.ledger.records.iter().map(|(id, info)| (*id, (info.clone(), full.ledger.transactions.of_record(*id).cloned().collect()))).collect();
        let record_of = |label: &str| b.tx[label].record;
        let mut present: BTreeMap<RecordId, (RecordInfo, Vec<Transaction>)> = BASE.iter().map(|l| (record_of(l), whole[&record_of(l)].clone())).collect();
        let inputs_now = |present: &BTreeMap<RecordId, (RecordInfo, Vec<Transaction>)>| -> Inputs {
            let mut i = full.clone();
            let txs: Vec<Transaction> = present.values().flat_map(|(_, t)| t.iter().cloned()).collect();
            let ids: BTreeSet<TransactionId> = txs.iter().map(|t| t.id.clone()).collect();
            i.ledger.transactions = txs.into_iter().collect();
            i.ledger.records = present.iter().map(|(r, (info, _))| (*r, info.clone())).collect();
            i.ledger.transfer_links = full.ledger.transfer_links.iter().filter(|(o, n)| ids.contains(o) && ids.contains(n)).map(|(o, n)| (o.clone(), n.clone())).collect();
            i
        };
        let mut e = Engine::build(inputs_now(&present));
        for (n, step) in steps().into_iter().enumerate() {
            let before_fresh = Engine::build(e.inputs().clone());
            let mut d = LedgerDelta::default();
            let records: BTreeSet<RecordId> = step.records.iter().map(|l| record_of(l)).collect();
            for r in records {
                present.insert(r, whole[&r].clone());
                d.records.insert(r, Some(whole[&r].clone()));
            }
            let now = inputs_now(&present);
            // only the engine's own apply counts: the builds beside it match whole
            bagholder_engine::engine::take_paths();
            let moved = e.apply(Change::Ledger(d));
            let paths = bagholder_engine::engine::take_paths();
            let family = e.family_of(Node::Instrument(named[step.on]));
            let took: Vec<Took> = paths.iter().filter(|(f, _)| *f == family).map(|(_, p)| *p).collect();
            let label = format!("seed {seed}, step {n} ({:?}, {:?})", step.records, step.path);
            match step.path {
                Path::Placed => {
                    assert_eq!(took, vec![Took::Placed], "{label}: the round trip is placed, and only that");
                    placed_steps += 1;
                }
                Path::Continued => assert_eq!(took, vec![Took::Continued], "{label}: continued, and only that"),
                Path::FromCut => {
                    assert_eq!(took, vec![Took::FromCut], "{label}: walked on from the cut, and only that");
                    cut_steps += 1;
                }
                Path::Whole(why) => assert!(!took.is_empty() && took.iter().all(|p| *p == Took::Whole), "{label}: matched whole ({why}), took {took:?}"),
            }
            let fresh = Engine::build(now.clone());
            let off = e.differences(&fresh);
            assert!(off.is_empty(), "{label}: the figures differ from a fresh build in {:?}", off.0);
            if let Some(why) = e.match_disagrees(&fresh) {
                panic!("{label}: {why}");
            }
            assert!(e.figures().matched == &match_lots(&now), "{label}: the families' match is not the whole ledger's");
            assert_eq!(moved, fresh.differences(&before_fresh), "{label}: what moved is not what differs");
            assert_eq!(by_side(&e), by_side(&fresh), "{label}: a payment answers a side filter by another round trip than a fresh build's");
        }
    }
    assert!(placed_steps >= 10 * 11, "only {placed_steps} steps were placed");
    assert!(cut_steps >= 10 * 11, "only {cut_steps} steps were walked on from a cut");
}
