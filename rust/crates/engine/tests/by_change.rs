//! A change to the record applied by key equals a fresh build, for every kind of
//! book (`docs/plans/stage-3-the-record-by-change.md`, B; brief 26, 4): generated
//! books holding every way the match couples holdings (an instrument across
//! accounts, contracts on an underlying, transfers between accounts, one whose
//! two sides are two instruments, corporate events whose legs name other
//! instruments, a row naming no instrument of a record whose legs are not
//! stated, a record whose legs are two uncoupled instruments at one instant
//! beside a sale of one of them, cash moved) have their records added, revised
//! and removed in a random order, and after every step the engine equals one
//! built fresh from the same inputs: its figures, its match whole and the
//! trades' identity; what it says moved is what differs; and its match is the
//! whole ledger's matched in one pass.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use bagholder_core::transaction::Transaction;
use bagholder_core::{RecordId, TransactionId};
use bagholder_engine::input::{Adjustment, Adjustments, Inputs, LedgerDelta, RecordInfo};
use bagholder_engine::ledger::match_lots;
use bagholder_engine::{Change, Engine};
use common::*;

/// splitmix64: the same seed, the same book.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
}

fn generate(seed: u64) -> Value {
    let mut r = Rng(seed);
    let mut tx: Vec<Value> = Vec::new();
    let mut adjustments: Vec<Value> = Vec::new();
    let mut links: Vec<Value> = Vec::new();
    let steps = 10 + r.below(40);
    let mut day = 0u32;
    for n in 0..steps {
        day += r.below(4) as u32;
        let date = format!("2026-{:02}-{:02}", 1 + day / 28, 1 + day % 28);
        let at = |m: u64| format!("{date}T15:{:02}:00Z", m % 60);
        let id = format!("t{n}");
        let acct = r.pick(&["A", "B", "D"]);
        match r.below(16) {
            0..=3 => {
                let i = r.pick(&["X", "W", "V"]);
                let q = 1 + r.below(50);
                tx.push(json!({"id": id, "account": acct, "day": date, "at": at(r.below(3)), "kind": "buy", "instrument": i, "qty": q.to_string(), "cash": format!("-{}", q * (5 + r.below(40)))}));
            }
            4..=5 => {
                let i = r.pick(&["X", "W", "V"]);
                let q = 1 + r.below(50);
                tx.push(json!({"id": id, "account": acct, "day": date, "at": at(r.below(3)), "kind": "sell", "instrument": i, "qty": format!("-{q}"), "cash": (q * (5 + r.below(40))).to_string()}));
            }
            6 => {
                // a coin bought or sold in the crypto account
                let q = 1 + r.below(900_000);
                let (kind, qty, cash) = if r.below(2) == 0 { ("buy", format!("0.{q:06}"), format!("-{}", 1 + r.below(500))) } else { ("sell", format!("-0.{q:06}"), (1 + r.below(500)).to_string()) };
                tx.push(json!({"id": id, "account": "C", "day": date, "at": at(1), "kind": kind, "instrument": "K", "qty": qty, "cash": cash}));
            }
            7 => {
                // a contract on X or W opened, closed, expired or assigned
                let (c, under) = if r.below(2) == 0 { ("C1", "X") } else { ("P1", "W") };
                let _ = under;
                match r.below(4) {
                    0 => tx.push(json!({"id": id, "account": "A", "day": date, "at": at(2), "kind": "sell", "effect": "open", "instrument": c, "qty": "-1", "cash": (10 + r.below(300)).to_string()})),
                    1 => tx.push(json!({"id": id, "account": "A", "day": date, "at": at(2), "kind": "buy", "effect": "close", "instrument": c, "qty": "1", "cash": format!("-{}", 10 + r.below(300))})),
                    2 => tx.push(json!({"id": id, "account": "A", "day": date, "at": at(2), "kind": "option-expiry", "instrument": c, "qty": "1"})),
                    _ => tx.push(json!({"id": id, "account": "A", "day": date, "at": at(2), "kind": "option-assignment", "instrument": c, "qty": "1", "cash": "5000"})),
                }
            }
            8 => {
                // a holding moved between accounts
                let i = r.pick(&["X", "W"]);
                let to = if acct == "A" { "B" } else { "A" };
                let q = 1 + r.below(30);
                let (out, inn) = (format!("{id}o"), format!("{id}i"));
                tx.push(json!({"id": out, "account": acct, "day": date, "at": at(3), "kind": "transfer-out", "instrument": i, "qty": format!("-{q}")}));
                tx.push(json!({"id": inn, "account": to, "day": date, "at": at(4), "kind": "transfer-in", "instrument": i, "qty": q.to_string()}));
                links.push(json!({"out": out, "in": inn}));
            }
            9 => {
                // a coin sent out as one instrument and received as another
                let q = 1 + r.below(900_000);
                let (out, inn) = (format!("{id}o"), format!("{id}i"));
                tx.push(json!({"id": out, "account": "C", "day": date, "at": at(3), "kind": "transfer-out", "instrument": "K", "qty": format!("-0.{q:06}")}));
                tx.push(json!({"id": inn, "account": "D", "day": date, "at": at(4), "kind": "transfer-in", "instrument": "K2", "qty": format!("0.{q:06}")}));
                links.push(json!({"out": out, "in": inn}));
            }
            10 => {
                // an event on X: a split, a spin-off into Y, or one whose parent is W
                tx.push(json!({"id": id, "account": "B", "day": date, "at": at(5), "kind": "corporate-event", "instrument": "X"}));
                let leg = match r.below(3) {
                    0 => json!({"from": "X", "to": "X", "units_per_unit": "2"}),
                    1 => json!({"from": "X", "to": "Y", "units_per_unit": "0.5", "cost_share": "0.2"}),
                    _ => json!({"from": "W", "to": "Y", "units_per_unit": "0.25", "cost_share": "0.1"}),
                };
                if r.below(3) != 0 {
                    adjustments.push(json!({"applies_to": id, "legs": [leg]}));
                }
            }
            11 => {
                // a row naming no instrument of a record whose legs are not stated
                tx.push(json!({"id": id, "account": "D", "day": date, "at": at(1), "kind": "buy", "cash": format!("-{}", 1 + r.below(900)), "problems": ["leg-unstated"]}));
            }
            12 => {
                // a record whose legs are two uncoupled instruments at one instant,
                // beside a sale of one of them
                let rec = format!("r{n}");
                tx.push(json!({"id": format!("{id}a"), "record": rec, "leg": "out", "account": "B", "day": date, "at": at(6), "kind": "corporate-event", "instrument": "X", "qty": "-10"}));
                tx.push(json!({"id": format!("{id}b"), "record": rec, "leg": "in", "account": "B", "day": date, "at": at(7), "kind": "corporate-event", "instrument": "V", "qty": "5"}));
                tx.push(json!({"id": format!("{id}s"), "account": "B", "day": date, "at": at(6), "kind": "sell", "instrument": "X", "qty": "-10", "cash": "100"}));
                // and a sale of the other between the record's two legs: the record
                // is applied at its first leg, before it
                tx.push(json!({"id": format!("{id}v"), "account": "B", "day": date, "at": format!("{date}T15:06:30Z"), "kind": "sell", "instrument": "V", "qty": "-5", "cash": "50"}));
            }
            _ => {
                // cash moved
                let kind = r.pick(&["deposit", "withdrawal", "interest"]);
                let amount = 1 + r.below(5000);
                let cash = if kind == "withdrawal" { format!("-{amount}") } else { amount.to_string() };
                tx.push(json!({"id": id, "account": acct, "day": date, "at": at(0), "kind": kind, "cash": cash}));
            }
        }
    }
    json!({
        "today": "2026-12-31",
        "accounts": [{"id": "A", "kind": "margin"}, {"id": "B"}, {"id": "C", "coin_minimum": "1.00 CAD"}, {"id": "D"}],
        "instruments": [
            {"id": "X"}, {"id": "W"}, {"id": "V"}, {"id": "Y"},
            {"id": "K", "kind": "crypto"}, {"id": "K2", "kind": "crypto"},
            {"id": "C1", "kind": "option", "underlying": "X", "expiry": "2026-06-19", "strike": "20", "right": "call", "multiplier": "100"},
            {"id": "P1", "kind": "option", "underlying": "W", "expiry": "2026-09-18", "strike": "20", "right": "put", "multiplier": "100"}
        ],
        "transactions": tx,
        "adjustments": adjustments,
        "transfer_links": links,
    })
}

/// The record as it stands in a step: each record present, as it reads now.
type Present = BTreeMap<RecordId, (RecordInfo, Vec<Transaction>)>;

/// The inputs of the full book with only the records `present` (and the links
/// and adjustments of what is there), built apart from the engine's own reading
/// of a change.
fn inputs_of(full: &Inputs, all_links: &[(TransactionId, TransactionId)], all_adjustments: &[Adjustment], present: &Present) -> Inputs {
    let mut i = full.clone();
    let txs: Vec<Transaction> = present.values().flat_map(|(_, t)| t.iter().cloned()).collect();
    let ids: BTreeSet<TransactionId> = txs.iter().map(|t| t.id.clone()).collect();
    i.ledger.transactions = txs.into_iter().collect();
    i.ledger.records = present.iter().map(|(r, (info, _))| (*r, info.clone())).collect();
    i.ledger.transfer_links = all_links.iter().filter(|(o, n)| ids.contains(o) && ids.contains(n)).cloned().collect();
    i.facts.adjustments = Adjustments::choose(all_adjustments.iter().filter(|a| ids.contains(&a.applies_to)).cloned());
    i
}

/// The delta that puts `r` in its state in `present` (gone where absent), with
/// every link on it as the inputs link them.
fn delta_for(r: RecordId, present: &Present, inputs: &Inputs) -> LedgerDelta {
    let mut d = LedgerDelta::default();
    d.records.insert(r, present.get(&r).cloned());
    d.links = inputs.ledger.transfer_links.iter().filter(|(o, n)| o.record == r || n.record == r).map(|(o, n)| (o.clone(), n.clone())).collect();
    d
}

#[test]
fn records_added_revised_and_removed_in_any_order_equal_a_fresh_build() {
    let mut steps = 0;
    for seed in 1..=160u64 {
        let case = generate(seed);
        let full = build(&case).inputs;
        let all_links: Vec<(TransactionId, TransactionId)> = full.ledger.transfer_links.iter().map(|(o, n)| (o.clone(), n.clone())).collect();
        let all_adjustments: Vec<Adjustment> = full.facts.adjustments.iter().map(|(_, a)| a.clone()).collect();
        let mut records: Vec<RecordId> = full.ledger.records.keys().copied().collect();
        let mut r = Rng(seed * 7919);
        for k in (1..records.len()).rev() {
            records.swap(k, r.below(k as u64 + 1) as usize);
        }
        let whole: Present = full.ledger.records.iter().map(|(id, info)| (*id, (info.clone(), full.ledger.transactions.of_record(*id).cloned().collect()))).collect();
        let mut present = Present::new();
        let mut e = Engine::build(inputs_of(&full, &all_links, &all_adjustments, &present));
        let mut plan: Vec<(RecordId, u8)> = records.iter().map(|id| (*id, 0)).collect();
        // some records revised after they arrive, some removed and some of those back
        for id in &records {
            if r.below(4) == 0 {
                let at = r.below(plan.len() as u64 + 1) as usize;
                plan.insert(at, (*id, 1));
            }
        }
        for id in &records {
            if r.below(5) == 0 {
                plan.push((*id, 2));
                if r.below(2) == 0 {
                    plan.push((*id, 0));
                }
            }
        }
        for (id, what) in plan {
            match what {
                0 => {
                    present.insert(id, whole[&id].clone());
                }
                1 => {
                    let Some((info, txs)) = present.get(&id).cloned() else { continue };
                    // a revision: the record's cash and units stated anew
                    let revised: Vec<Transaction> = txs
                        .into_iter()
                        .map(|mut t| {
                            t.cash = t.cash.map(|c| bagholder_core::Money::new(c.amount.checked_add(bagholder_core::Dec::ONE).unwrap(), c.currency));
                            t
                        })
                        .collect();
                    present.insert(id, (info, revised));
                }
                _ => {
                    present.remove(&id);
                }
            }
            let now = inputs_of(&full, &all_links, &all_adjustments, &present);
            let before_fresh = Engine::build(e.inputs().clone());
            let mut d = delta_for(id, &present, &now);
            // the adjustments of the record's rows, as the inputs hold them now
            for t in whole[&id].1.iter() {
                d.adjustments.insert(t.id.clone(), if present.contains_key(&id) { all_adjustments.iter().filter(|a| a.applies_to == t.id).cloned().collect() } else { vec![] });
            }
            let moved = e.apply(Change::Ledger(d));
            steps += 1;
            let fresh = Engine::build(now.clone());
            let off = e.differences(&fresh);
            assert!(off.is_empty(), "book {seed}, record {id}: the figures differ from a fresh build in {:?}", off.0);
            if let Some(why) = e.match_disagrees(&fresh) {
                panic!("book {seed}, record {id}: {why}");
            }
            assert!(e.figures().matched == &match_lots(&now), "book {seed}, record {id}: the families' match is not the whole ledger's");
            let expected = fresh.differences(&before_fresh);
            assert_eq!(moved, expected, "book {seed}, record {id}: what moved is not what differs");
        }
    }
    assert!(steps > 2_000, "only {steps} steps were tried");
}
