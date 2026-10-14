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
        match r.below(17) {
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
            9 | 14 if r.below(4) != 0 => {
                // a holding moved to another account whose row names no instrument:
                // the units land under the instrument sent (common, so that the
                // removal of its sending side, which leaves the receiving account
                // holding none, is met in many books: brief 28, E)
                let q = 1 + r.below(30);
                let (out, inn) = (format!("{id}o"), format!("{id}i"));
                tx.push(json!({"id": out, "account": "A", "day": date, "at": at(3), "kind": "transfer-out", "instrument": "X", "qty": format!("-{q}")}));
                tx.push(json!({"id": inn, "account": "E", "day": date, "at": at(4), "kind": "transfer-in", "qty": q.to_string()}));
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
            13 => {
                // an event paying two stock dividends, on X and on W: two rows of one event
                tx.push(json!({"id": format!("{id}x"), "account": "B", "day": date, "at": at(1), "kind": "buy", "instrument": "X", "qty": "10", "cash": "-100"}));
                tx.push(json!({"id": format!("{id}w"), "account": "B", "day": date, "at": at(1), "kind": "buy", "instrument": "W", "qty": "10", "cash": "-100"}));
                tx.push(json!({"id": id, "account": "B", "day": date, "at": at(5), "kind": "corporate-event", "instrument": "X"}));
                adjustments.push(json!({"applies_to": id, "legs": [
                    {"from": "X", "to": "X", "units_per_unit": "0.1", "cost": "12.50"},
                    {"from": "W", "to": "W", "units_per_unit": "0.05", "cost": "3.25"}
                ]}));
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
        "accounts": [{"id": "A", "kind": "margin"}, {"id": "B"}, {"id": "C", "coin_minimum": "1.00 CAD"}, {"id": "D"}, {"id": "E"}],
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


/// What the person and the broker keep beside the records, as a step leaves it.
#[derive(Clone)]
struct Kept {
    present: Present,
    trades: BTreeMap<bagholder_core::TradeId, (bagholder_engine::input::Rank, bagholder_core::journal::Trade)>,
    groups: BTreeMap<bagholder_core::GroupId, (bagholder_engine::input::Rank, bagholder_core::journal::Group)>,
    instruments: BTreeMap<bagholder_core::InstrumentId, bagholder_engine::input::InstrumentInfo>,
    accounts: BTreeMap<bagholder_core::AccountId, bagholder_engine::input::AccountInfo>,
    adjustments: BTreeMap<TransactionId, Vec<Adjustment>>,
}

/// The inputs a step leaves, built apart from the engine's reading of a change.
fn inputs_kept(full: &Inputs, all_links: &[(TransactionId, TransactionId)], k: &Kept) -> Inputs {
    let mut i = full.clone();
    let txs: Vec<Transaction> = k.present.values().flat_map(|(_, t)| t.iter().cloned()).collect();
    let ids: BTreeSet<TransactionId> = txs.iter().map(|t| t.id.clone()).collect();
    i.ledger.transactions = txs.into_iter().collect();
    i.ledger.records = k.present.iter().map(|(r, (info, _))| (*r, info.clone())).collect();
    i.ledger.transfer_links = all_links.iter().filter(|(o, n)| ids.contains(o) && ids.contains(n)).cloned().collect();
    i.ledger.trades = k.trades.values().cloned().collect();
    i.ledger.groups = k.groups.values().cloned().collect();
    i.ledger.instruments = k.instruments.clone();
    i.ledger.accounts = k.accounts.clone();
    i.facts.adjustments = Adjustments::choose(k.adjustments.values().flatten().filter(|a| ids.contains(&a.applies_to)).cloned());
    i
}

/// Every kind of write the book makes, on generated books with broker
/// statements for every account, applied in a random order: records added,
/// revised (a row moved to another instrument, account, quantity or kind; a
/// record's legs stated or not), removed and back; trades opened and gone;
/// groups across instruments made and gone; an adjustment set or cleared
/// alone; an instrument's name or terms, or an account's management, changed
/// alone. After every step the engine equals a fresh build in every figure, in
/// the order shown, in its match, identity and families, and its match is the
/// whole ledger's; what moved is what differs (brief 27, 3 and 11).
#[test]
fn every_kind_of_write_equals_a_fresh_build_in_every_figure_and_order() {
    let mut steps = 0;
    for seed in 1..=60u64 {
        let case = generate(seed);
        let mut b = build(&case);
        let mut full = b.inputs.clone();
        // the broker states every account's cash and units, as of today
        let instruments: Vec<bagholder_core::InstrumentId> = full.ledger.instruments.keys().copied().collect();
        let mut r = Rng(seed * 104729);
        for a in full.ledger.accounts.keys().copied().collect::<Vec<_>>() {
            let mut held: BTreeMap<_, _> = (0..r.below(3)).map(|_| (instruments[r.below(instruments.len() as u64) as usize], bagholder_core::Dec::from_int(r.below(60) as i64))).collect();
            // the account that only ever receives X: the broker states what it holds of it
            if a == b.ids.account("E") {
                held.insert(b.ids.instrument("X"), bagholder_core::Dec::from_int(10));
            }
            let cash = BTreeMap::from([(bagholder_core::Currency::CAD, bagholder_core::Dec::from_int(r.below(5000) as i64))]);
            let stated = bagholder_engine::input::BrokerAccount {
                cash: cash.clone(),
                cash_read: Some(cash),
                held,
                held_as_of: Some(full.clock.today),
                as_of: Some(full.clock.now),
                activity_read_at: Some(full.clock.now),
                ..Default::default()
            };
            full.market.brokers.insert(a, stated);
        }
        let all_links: Vec<(TransactionId, TransactionId)> = full.ledger.transfer_links.iter().map(|(o, n)| (o.clone(), n.clone())).collect();
        let whole: Present = full.ledger.records.iter().map(|(id, info)| (*id, (info.clone(), full.ledger.transactions.of_record(*id).cloned().collect()))).collect();
        let mut by_tx: BTreeMap<TransactionId, Vec<Adjustment>> = BTreeMap::new();
        for (_, a) in full.facts.adjustments.iter() {
            by_tx.entry(a.applies_to.clone()).or_default().push(a.clone());
        }
        let mut k = Kept { present: Present::new(), trades: BTreeMap::new(), groups: BTreeMap::new(), instruments: full.ledger.instruments.clone(), accounts: full.ledger.accounts.clone(), adjustments: by_tx.clone() };
        let mut e = Engine::build(inputs_kept(&full, &all_links, &k));
        let mut rank = 0usize;
        let records: Vec<RecordId> = whole.keys().copied().collect();
        for step in 0..(3 * records.len()) {
            let before_fresh = Engine::build(e.inputs().clone());
            let mut d = LedgerDelta::default();
            match r.below(10) {
                0..=2 => {
                    // a record arrives, again or anew
                    let id = records[r.below(records.len() as u64) as usize];
                    k.present.insert(id, whole[&id].clone());
                    d.records.insert(id, Some(whole[&id].clone()));
                }
                3 => {
                    // a record revised: a row moved, or its legs stated or not
                    let Some(id) = k.present.keys().nth(r.below(k.present.len().max(1) as u64) as usize).copied() else { continue };
                    let (mut info, mut txs) = k.present[&id].clone();
                    let n = r.below(txs.len().max(1) as u64) as usize;
                    match r.below(5) {
                        0 => txs[n].instrument = txs[n].instrument.map(|_| instruments[r.below(4) as usize]),
                        1 => txs[n].account = *k.accounts.keys().nth(r.below(k.accounts.len() as u64) as usize).unwrap(),
                        2 => txs[n].quantity = txs[n].quantity.map(|q| q.checked_add(bagholder_core::Dec::ONE).unwrap()),
                        3 => {
                            use bagholder_core::transaction::Kind;
                            txs[n].kind = match txs[n].kind {
                                Kind::Buy => Kind::Sell,
                                Kind::Sell => Kind::Buy,
                                other => other,
                            }
                        }
                        _ => {
                            if info.problems.iter().any(|p| p.code == "leg-unstated") {
                                info.problems.retain(|p| p.code != "leg-unstated");
                            } else {
                                info.problems.push(bagholder_core::record::Problem::new("leg-unstated", "stated by the test"));
                            }
                        }
                    }
                    k.present.insert(id, (info.clone(), txs.clone()));
                    d.records.insert(id, Some((info, txs)));
                }
                4 => {
                    // a record gone: as often as not a transfer's sending side
                    let outs: Vec<RecordId> = k.present.iter().filter(|(_, (_, t))| t.iter().any(|t| t.kind == bagholder_core::transaction::Kind::TransferOut)).map(|(id, _)| *id).collect();
                    let id = if !outs.is_empty() && r.below(2) == 0 {
                        outs[r.below(outs.len() as u64) as usize]
                    } else {
                        let Some(id) = k.present.keys().nth(r.below(k.present.len().max(1) as u64) as usize).copied() else { continue };
                        id
                    };
                    k.present.remove(&id);
                    d.records.insert(id, None);
                }
                5 => {
                    // a trade opened on a purchase, or one gone
                    let buys: Vec<(TransactionId, bagholder_core::InstrumentId)> =
                        k.present.values().flat_map(|(_, t)| t.iter()).filter(|t| t.kind == bagholder_core::transaction::Kind::Buy).filter_map(|t| t.instrument.map(|i| (t.id.clone(), i))).collect();
                    if r.below(3) == 0 && !k.trades.is_empty() {
                        let t = *k.trades.keys().nth(r.below(k.trades.len() as u64) as usize).unwrap();
                        k.trades.remove(&t);
                        d.trades.insert(t, None);
                    } else if let Some((opening, instrument)) = buys.get(r.below(buys.len().max(1) as u64) as usize).cloned() {
                        let id = b.ids.trade(&format!("t-{seed}-{step}"));
                        rank += 1;
                        let t = bagholder_core::journal::Trade { id, anchor: bagholder_core::journal::Anchor::Opening(bagholder_core::journal::Opening { transaction: opening, instrument }), legacy_key: None };
                        k.trades.insert(id, (rank_of(rank), t.clone()));
                        d.trades.insert(id, Some((rank_of(rank), t)));
                    } else {
                        continue;
                    }
                }
                6 => {
                    // a group of two trades, whatever they are on, or one gone
                    if r.below(3) == 0 && !k.groups.is_empty() {
                        let g = *k.groups.keys().nth(r.below(k.groups.len() as u64) as usize).unwrap();
                        k.groups.remove(&g);
                        d.groups.insert(g, None);
                    } else if k.trades.len() >= 2 {
                        let ts: Vec<bagholder_core::TradeId> = k.trades.keys().copied().collect();
                        let (a, c) = (ts[r.below(ts.len() as u64) as usize], ts[r.below(ts.len() as u64) as usize]);
                        let id = b.ids.group(&format!("g-{seed}-{step}"));
                        rank += 1;
                        let g = bagholder_core::journal::Group { id, locked: true, members: vec![a, c] };
                        k.groups.insert(id, (rank_of(rank), g.clone()));
                        d.groups.insert(id, Some((rank_of(rank), g)));
                    } else {
                        continue;
                    }
                }
                7 => {
                    // an adjustment set or cleared alone
                    let events: Vec<TransactionId> = k.present.values().flat_map(|(_, t)| t.iter()).filter(|t| t.kind == bagholder_core::transaction::Kind::CorporateEvent).map(|t| t.id.clone()).collect();
                    let Some(tx) = events.get(r.below(events.len().max(1) as u64) as usize).cloned() else { continue };
                    let now = if k.adjustments.get(&tx).is_some_and(|l| !l.is_empty()) {
                        vec![]
                    } else {
                        let y = instruments[3];
                        vec![Adjustment { applies_to: tx.clone(), legs: vec![bagholder_engine::input::AdjustmentLeg { from: Some(instruments[0]), to: Some(y), units_per_unit: Some(bagholder_core::Dec::ONE), cost_share: Some(bagholder_core::Dec::parse("0.3").unwrap()), ..Default::default() }], source: bagholder_core::SourceName::person() }]
                    };
                    k.adjustments.insert(tx.clone(), now.clone());
                    d.adjustments.insert(tx, now);
                }
                8 => {
                    // an instrument's name or a contract's terms changed alone
                    let i = instruments[r.below(instruments.len() as u64) as usize];
                    let mut info = k.instruments[&i].clone();
                    match info.terms.as_mut() {
                        Some(t) if r.below(2) == 0 => t.multiplier = Some(bagholder_core::Dec::from_int(if t.multiplier == Some(bagholder_core::Dec::from_int(10)) { 100 } else { 10 })),
                        _ => {
                            if let Some(n) = info.names.last_mut() {
                                n.symbol = format!("{}X", n.symbol);
                            }
                        }
                    }
                    k.instruments.insert(i, info.clone());
                    d.instruments.insert(i, Some(info));
                }
                _ => {
                    // an account managed by its broker, or no longer
                    let a = *k.accounts.keys().nth(r.below(k.accounts.len() as u64) as usize).unwrap();
                    let mut info = k.accounts[&a].clone();
                    if let bagholder_core::account::AccountType::Known { managed, .. } = &mut info.account.account_type {
                        *managed = !*managed;
                    }
                    k.accounts.insert(a, info.clone());
                    d.accounts.insert(a, Some(info));
                }
            }
            let now = inputs_kept(&full, &all_links, &k);
            // the links on every record named, as the inputs link them now
            let named: BTreeSet<RecordId> = d.records.keys().copied().collect();
            d.links = now.ledger.transfer_links.iter().filter(|(o, n)| named.contains(&o.record) || named.contains(&n.record)).map(|(o, n)| (o.clone(), n.clone())).collect();
            // and the adjustments of every row of a record named
            for id in &named {
                for t in whole[id].1.iter() {
                    d.adjustments.entry(t.id.clone()).or_insert_with(|| if k.present.contains_key(id) { k.adjustments.get(&t.id).cloned().unwrap_or_default() } else { vec![] });
                }
            }
            let moved = e.apply(Change::Ledger(d));
            steps += 1;
            let fresh = Engine::build(now.clone());
            let off = e.differences(&fresh);
            assert!(off.is_empty(), "book {seed}, step {step}: the figures differ from a fresh build in {:?}", off.0);
            if let Some(why) = e.match_disagrees(&fresh) {
                panic!("book {seed}, step {step}: {why}");
            }
            assert!(e.figures().matched == &match_lots(&now), "book {seed}, step {step}: the families' match is not the whole ledger's");
            assert_eq!(moved, fresh.differences(&before_fresh), "book {seed}, step {step}: what moved is not what differs");
        }
    }
    assert!(steps > 2_000, "only {steps} steps were tried");
}

fn rank_of(n: usize) -> bagholder_engine::input::Rank {
    (format!("z{n:08}"), n as i64)
}
