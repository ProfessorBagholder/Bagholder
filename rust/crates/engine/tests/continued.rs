//! A family's match continued from its kept state equals the family matched
//! whole, and is used exactly where the guards allow it
//! (`docs/plans/stage-3-a-change-on-a-deep-history.md`, B and C). Each step of a
//! sequence is labelled by the test itself with the path it expects for the
//! family it touches (continued, or matched whole and why), never by asking the
//! guard; the engine's record of the path each family took must match the label,
//! and after every step the engine equals a fresh build in every figure, in the
//! order shown, in its match and identity, and what moved is what differs.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use bagholder_core::transaction::Transaction;
use bagholder_core::{AccountId, InstrumentId, RecordId, TransactionId};
use bagholder_engine::families::Node;
use bagholder_engine::input::{Adjustment, Adjustments, Inputs, LedgerDelta, RecordInfo};
use bagholder_engine::ledger::match_lots;
use bagholder_engine::{Change, Engine};
use common::*;

/// splitmix64: the same seed, the same sequence.
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
}

/// What a step expects of the family of the instrument it names.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Path {
    Continued,
    /// Matched whole, and the reason the test knows it must be.
    Whole(&'static str),
    /// Nothing matched (a trade, a group, a journal entry).
    None,
}

/// One step: the records it adds (by label), what else it changes, and what it
/// expects of the family of `on`.
struct Step {
    records: Vec<&'static str>,
    on: &'static str,
    path: Path,
    extra: Extra,
}

#[derive(Clone, Copy)]
enum Extra {
    None,
    /// The account `N` comes into the book with the step.
    NewAccount,
    /// The contract O's multiplier changes with the step.
    Terms,
    /// The crypto account's smallest coin order rises with the step.
    CoinMinimum,
    /// The contract Q, on S, comes into the book with the step.
    NewContract,
    /// A trade opened on a row's fill (an add-on purchase of S), or the first gone.
    TradeOn(&'static str),
    TradeGone,
    /// A group of the trades on S and T.
    Group,
    /// The day turns.
    Clock,
    /// The event row `e1` is given a split of S.
    Split,
    /// The row `c1` is given its cost, from a leg naming T.
    CostFromT,
    /// The deposit `w1` is revised to a row whose record's legs are not stated:
    /// everything account A holds becomes one family.
    Unstated,
    /// A record of the family revised (its quantity changed) in the same change.
    Revise(&'static str),
    /// A record of the family removed in the same change.
    Remove(&'static str),
    /// The row `a5` is given an adjustment whose legs name only T.
    AdjustNamingT,
}

/// The book every sequence is written in: the base rows, and every row a step
/// adds, each its own record. Amounts and times vary by seed; which path each
/// step must take does not.
fn universe(seed: u64) -> Value {
    let mut r = Rng(seed);
    let q = |r: &mut Rng| (1 + r.below(40)).to_string();
    let cash = |r: &mut Rng| (10 + r.below(900)).to_string();
    let mut tx: Vec<Value> = Vec::new();
    let mut add = |v: Value| tx.push(v);
    // the base: shares across accounts, a transfer linked and one not, a contract
    // opened and closed, one left open past its expiry, a coin
    add(json!({"id": "s1", "account": "A", "day": "2026-01-05", "at": "2026-01-05T15:00:00Z", "kind": "buy", "instrument": "S", "qty": "100", "cash": format!("-{}", cash(&mut r))}));
    add(json!({"id": "s2", "account": "B", "day": "2026-01-06", "at": "2026-01-06T15:00:00Z", "kind": "buy", "instrument": "S", "qty": "50", "cash": format!("-{}", cash(&mut r))}));
    add(json!({"id": "s3", "account": "A", "day": "2026-02-10", "at": "2026-02-10T15:00:00Z", "kind": "sell", "instrument": "S", "qty": format!("-{}", q(&mut r)), "cash": cash(&mut r)}));
    add(json!({"id": "s4o", "account": "A", "day": "2026-03-02", "at": "2026-03-02T15:00:00Z", "kind": "transfer-out", "instrument": "S", "qty": "-10"}));
    add(json!({"id": "s4i", "account": "B", "day": "2026-03-02", "at": "2026-03-02T15:01:00Z", "kind": "transfer-in", "instrument": "S", "qty": "10"}));
    add(json!({"id": "s5o", "account": "A", "day": "2026-03-09", "at": "2026-03-09T15:00:00Z", "kind": "transfer-out", "instrument": "S", "qty": "-5"}));
    add(json!({"id": "t1", "account": "A", "day": "2026-01-07", "at": "2026-01-07T15:00:00Z", "kind": "buy", "instrument": "T", "qty": "200", "cash": format!("-{}", cash(&mut r))}));
    add(json!({"id": "o1", "account": "A", "day": "2026-04-01", "at": "2026-04-01T15:00:00Z", "kind": "sell", "effect": "open", "instrument": "O", "qty": "-1", "cash": cash(&mut r)}));
    add(json!({"id": "o2", "account": "A", "day": "2026-04-15", "at": "2026-04-15T15:00:00Z", "kind": "buy", "effect": "close", "instrument": "O", "qty": "1", "cash": format!("-{}", cash(&mut r))}));
    add(json!({"id": "u1", "account": "A", "day": "2026-01-08", "at": "2026-01-08T15:00:00Z", "kind": "buy", "instrument": "U", "qty": "10", "cash": "-100"}));
    add(json!({"id": "u2", "account": "A", "day": "2026-02-02", "at": "2026-02-02T15:00:00Z", "kind": "sell", "effect": "open", "instrument": "O2", "qty": "-1", "cash": "40"}));
    add(json!({"id": "k1", "account": "C", "day": "2026-01-09", "at": "2026-01-09T15:00:00Z", "kind": "buy", "instrument": "K", "qty": "0.123456", "cash": "-50"}));
    add(json!({"id": "w1", "account": "A", "day": "2026-01-02", "at": "2026-01-02T15:00:00Z", "kind": "deposit", "cash": "5000"}));
    add(json!({"id": "k2", "account": "C", "day": "2026-06-30", "at": "2026-06-30T15:00:00Z", "kind": "sell", "instrument": "K", "qty": "-0.100000", "cash": "45"}));
    add(json!({"id": "x0", "account": "A", "day": "2026-01-03", "at": "2026-01-03T15:00:00Z", "kind": "buy", "instrument": "X", "qty": "1", "cash": "-10"}));
    add(json!({"id": "y0", "account": "A", "day": "2026-01-05", "at": "2026-01-05T15:00:00Z", "kind": "buy", "instrument": "Y", "qty": "5", "cash": "-50"}));
    // the steps' rows
    add(json!({"id": "a1", "account": "A", "day": "2026-07-01", "kind": "buy", "instrument": "S", "qty": q(&mut r), "cash": format!("-{}", cash(&mut r))}));
    add(json!({"id": "a2", "account": "B", "day": "2026-07-02", "at": "2026-07-02T15:00:00Z", "kind": "sell", "instrument": "S", "qty": "-3", "cash": cash(&mut r)}));
    add(json!({"id": "a3", "account": "A", "day": "2026-02-01", "at": "2026-02-01T15:00:00Z", "kind": "buy", "instrument": "S", "qty": "2", "cash": "-30"}));
    add(json!({"id": "k3", "account": "C", "day": "2026-07-03", "at": "2026-07-03T15:00:00Z", "kind": "buy", "instrument": "K", "qty": "0.500000", "cash": "-200"}));
    add(json!({"id": "k4", "account": "C", "day": "2026-07-04", "at": "2026-07-04T15:00:00Z", "kind": "buy", "instrument": "K", "qty": "0.1234567", "cash": "-50"}));
    add(json!({"id": "k5", "account": "C", "day": "2026-07-05", "kind": "buy", "instrument": "K", "qty": "0.12345678", "cash": "-50", "source": "person"}));
    add(json!({"id": "a4o", "account": "A", "day": "2026-07-06", "at": "2026-07-06T15:00:00Z", "kind": "transfer-out", "instrument": "S", "qty": "-4"}));
    add(json!({"id": "a4i", "account": "B", "day": "2026-07-06", "at": "2026-07-06T15:01:00Z", "kind": "transfer-in", "instrument": "S", "qty": "4"}));
    add(json!({"id": "n1", "account": "N", "day": "2026-07-07", "kind": "buy", "instrument": "S", "qty": "5", "cash": "-80"}));
    add(json!({"id": "o3", "account": "A", "day": "2026-07-10", "at": "2026-07-10T15:00:00Z", "kind": "sell", "effect": "open", "instrument": "O", "qty": "-1", "cash": cash(&mut r)}));
    add(json!({"id": "o4", "account": "A", "day": "2026-07-10", "at": "2026-07-10T15:00:00Z", "kind": "buy", "effect": "close", "instrument": "O", "qty": "1", "cash": format!("-{}", cash(&mut r))}));
    add(json!({"id": "o5", "account": "B", "day": "2026-07-10", "at": "2026-07-10T15:00:00Z", "kind": "buy", "effect": "close", "instrument": "O", "qty": "1", "cash": "-20"}));
    add(json!({"id": "o6", "account": "A", "day": "2026-07-20", "at": "2026-07-20T15:00:00Z", "kind": "option-expiry", "instrument": "O", "qty": "1"}));
    add(json!({"id": "u3", "account": "A", "day": "2026-07-21", "at": "2026-07-21T15:00:00Z", "kind": "buy", "instrument": "U", "qty": "1", "cash": "-10"}));
    add(json!({"id": "e1", "account": "A", "day": "2026-07-22", "at": "2026-07-22T15:00:00Z", "kind": "corporate-event", "instrument": "S"}));
    add(json!({"id": "s5i", "account": "B", "day": "2026-07-23", "at": "2026-07-23T15:00:00Z", "kind": "transfer-in", "instrument": "S", "qty": "5"}));
    add(json!({"id": "t2", "account": "A", "day": "2026-07-24", "at": "2026-07-24T15:00:00Z", "kind": "buy", "instrument": "T", "qty": "1", "cash": "-30"}));
    add(json!({"id": "k6", "account": "C", "day": "2026-07-25", "at": "2026-07-25T15:00:00Z", "kind": "buy", "instrument": "K", "qty": "0.010000", "cash": "-4"}));
    add(json!({"id": "q1", "account": "A", "day": "2026-07-26", "at": "2026-07-26T15:00:00Z", "kind": "sell", "effect": "open", "instrument": "Q", "qty": "-1", "cash": "25"}));
    add(json!({"id": "r1", "account": "A", "day": "2026-08-03", "at": "2026-08-03T15:00:00Z", "kind": "sell", "effect": "open", "instrument": "O3", "qty": "-2", "cash": cash(&mut r)}));
    add(json!({"id": "r2c", "record": "roll", "leg": "close", "account": "A", "day": "2026-08-10", "at": "2026-08-10T15:00:00Z", "kind": "buy", "effect": "close", "instrument": "O3", "qty": "1", "cash": "-15"}));
    add(json!({"id": "r2o", "record": "roll", "leg": "open", "account": "A", "day": "2026-08-10", "at": "2026-08-10T15:00:00Z", "kind": "sell", "effect": "open", "instrument": "O4", "qty": "-1", "cash": "18"}));
    add(json!({"id": "r3", "account": "A", "day": "2026-08-17", "at": "2026-08-17T15:00:00Z", "kind": "buy", "effect": "close", "instrument": "O4", "qty": "1", "cash": "-9"}));
    add(json!({"id": "a6", "account": "B", "day": "2026-08-21", "at": "2026-08-21T15:00:00Z", "kind": "buy", "instrument": "S", "qty": "1", "cash": "-14"}));
    add(json!({"id": "c1", "account": "B", "day": "2026-08-22", "at": "2026-08-22T15:00:00Z", "kind": "transfer-in", "instrument": "S", "qty": "3"}));
    add(json!({"id": "a7", "account": "B", "day": "2026-08-23", "at": "2026-08-23T15:00:00Z", "kind": "buy", "instrument": "S", "qty": "1", "cash": "-14"}));
    add(json!({"id": "a5", "account": "A", "day": "2026-08-20", "at": "2026-08-20T15:00:00Z", "kind": "buy", "instrument": "S", "qty": "1", "cash": "-14"}));
    // a later sale of S beside a revised record; a row at o3's instant on another holding
    add(json!({"id": "a8", "account": "A", "day": "2026-07-03", "at": "2026-07-03T15:00:00Z", "kind": "sell", "instrument": "S", "qty": "-2", "cash": "30"}));
    add(json!({"id": "t3", "account": "B", "day": "2026-07-10", "at": "2026-07-10T15:00:00Z", "kind": "buy", "instrument": "T", "qty": "1", "cash": "-20"}));
    // X bought at 15:00, then one record that day, untimed, selling X and buying Y:
    // each leg after its own family's last row, the record's earliest before X's
    add(json!({"id": "l1", "account": "A", "day": "2026-08-24", "at": "2026-08-24T15:00:00Z", "kind": "buy", "instrument": "X", "qty": "10", "cash": "-100"}));
    add(json!({"id": "bxs", "record": "bx", "leg": "sell", "account": "A", "day": "2026-08-24", "kind": "sell", "instrument": "X", "qty": "-10", "cash": "110"}));
    add(json!({"id": "bxb", "record": "bx", "leg": "buy", "account": "A", "day": "2026-08-24", "kind": "buy", "instrument": "Y", "qty": "5", "cash": "-60"}));
    // the put Q assigned
    add(json!({"id": "q2", "account": "A", "day": "2026-08-25", "at": "2026-08-25T21:30:00Z", "kind": "option-assignment", "instrument": "Q", "qty": "1", "cash": "-1000"}));
    add(json!({"id": "a9", "account": "B", "day": "2026-08-26", "at": "2026-08-26T15:00:00Z", "kind": "sell", "instrument": "S", "qty": "-1", "cash": "15"}));
    json!({
        "today": "2026-12-31",
        "accounts": [{"id": "A", "kind": "margin"}, {"id": "B"}, {"id": "C", "coin_minimum": "1.00 CAD"}, {"id": "N"}],
        "instruments": [
            {"id": "S"}, {"id": "T"}, {"id": "U"}, {"id": "X"}, {"id": "Y"}, {"id": "K", "kind": "crypto"},
            {"id": "O", "kind": "option", "underlying": "T", "expiry": "2027-06-18", "strike": "20", "right": "call", "multiplier": "100"},
            {"id": "O2", "kind": "option", "underlying": "U", "expiry": "2026-03-20", "strike": "20", "right": "call", "multiplier": "100"},
            {"id": "O3", "kind": "option", "underlying": "T", "expiry": "2027-09-17", "strike": "25", "right": "call", "multiplier": "100"},
            {"id": "O4", "kind": "option", "underlying": "T", "expiry": "2027-12-17", "strike": "30", "right": "call", "multiplier": "100"},
            {"id": "Q", "kind": "option", "underlying": "S", "expiry": "2027-06-18", "strike": "10", "right": "put", "multiplier": "100"}
        ],
        "transactions": tx,
        "transfer_links": [{"out": "s4o", "in": "s4i"}, {"out": "a4o", "in": "a4i"}, {"out": "s5o", "in": "s5i"}],
        "trades": [{"id": "ts", "opened_by": "s1", "instrument": "S"}, {"id": "tt", "opened_by": "t1", "instrument": "T"}],
    })
}

const BASE: &[&str] = &["s1", "s2", "s3", "s4o", "s4i", "s5o", "t1", "o1", "o2", "u1", "u2", "w1", "k1", "k2", "x0", "y0"];

fn steps() -> Vec<Step> {
    use Extra as E;
    use Path::*;
    let st = |records: Vec<&'static str>, on, path, extra| Step { records, on, path, extra };
    vec![
        st(vec!["a1"], "S", Continued, E::None),
        st(vec!["a2"], "S", Continued, E::None),
        st(vec![], "S", None, E::TradeOn("a1")),
        st(vec!["a8"], "S", Whole("a record of the family revised in the same change"), E::Revise("s3")),
        st(vec!["a3"], "S", Whole("a row placed before what the family matched"), E::None),
        st(vec!["k3"], "K", Continued, E::None),
        st(vec!["k4"], "K", Whole("a coin's row finer than its source's rows were"), E::None),
        st(vec!["k5"], "K", Continued, E::None),
        st(vec!["a4o", "a4i"], "S", Continued, E::None),
        st(vec!["n1"], "S", Continued, E::NewAccount),
        st(vec!["o3"], "T", Continued, E::None),
        // the stated open at the last instant is kept across a walk at that instant
        st(vec!["t3"], "T", Continued, E::None),
        st(vec!["o4"], "T", Whole("a stated close at the instant of a stated open walked on its holding"), E::None),
        st(vec!["o5"], "T", Continued, E::None),
        st(vec![], "S", None, E::Group),
        st(vec!["o6"], "T", Whole("an expiry row decides how an earlier lot ended"), E::None),
        st(vec!["u3"], "U", Whole("the family's expiry up to today acted"), E::None),
        st(vec!["e1"], "S", Whole("a corporate event"), E::None),
        st(vec!["s5i"], "S", Whole("a transfer linked to a row already walked"), E::None),
        st(vec!["t2"], "T", Whole("a contract's terms changed"), E::Terms),
        st(vec!["k6"], "K", Whole("an account's smallest coin order changed"), E::CoinMinimum),
        st(vec!["q1"], "S", Whole("a new contract joins its underlying's family"), E::NewContract),
        st(vec![], "S", None, E::TradeGone),
        st(vec!["r1"], "T", Continued, E::None),
        st(vec!["r2c", "r2o"], "T", Continued, E::None),
        // a trade on the old round trip after the roll: shown on the holding the roll moved to
        st(vec![], "O3", None, E::TradeOn("r1")),
        st(vec!["r3"], "T", Continued, E::None),
        st(vec![], "S", Whole("the day turned"), E::Clock),
        st(vec!["a5"], "S", Continued, E::None),
        st(vec!["a6"], "S", Whole("an adjustment set on a row the family walked"), E::Split),
        st(vec![], "T", Whole("an adjustment on another family's row names only this family"), E::AdjustNamingT),
        st(vec!["c1"], "S", Whole("a row's adjustment names an instrument of another family"), E::CostFromT),
        st(vec!["l1"], "X", Continued, E::None),
        st(vec!["bxs", "bxb"], "X", Whole("a record's earliest leg, on another family, is before what this family walked"), E::None),
        st(vec!["q2"], "S", Whole("an assignment decides how an earlier lot ended"), E::None),
        st(vec!["a9"], "S", Whole("a record of the family removed in the same change"), E::Remove("a2")),
        st(vec!["a7"], "S", Whole("a record made to wait joins the account's holdings into one family"), E::Unstated),
    ]
}

#[test]
fn a_continued_match_equals_a_whole_one_and_is_taken_where_the_guards_allow() {
    let mut continued_steps = 0;
    for seed in 1..=30u64 {
        let case = universe(seed);
        let mut b = build(&case);
        let full = b.inputs.clone();
        let all_links: Vec<(TransactionId, TransactionId)> = full.ledger.transfer_links.iter().map(|(o, n)| (o.clone(), n.clone())).collect();
        let whole: BTreeMap<RecordId, (RecordInfo, Vec<Transaction>)> = full.ledger.records.iter().map(|(id, info)| (*id, (info.clone(), full.ledger.transactions.of_record(*id).cloned().collect()))).collect();
        let record_of = |b: &Built, label: &str| b.tx[label].record;
        // what the book holds at a step, built apart from the engine
        let mut present: BTreeMap<RecordId, (RecordInfo, Vec<Transaction>)> = BASE.iter().map(|l| (record_of(&b, l), whole[&record_of(&b, l)].clone())).collect();
        let mut accounts = full.ledger.accounts.clone();
        accounts.remove(&b.ids.account("N"));
        let mut instruments = full.ledger.instruments.clone();
        instruments.remove(&b.ids.instrument("Q"));
        let mut trades = full.ledger.trades.clone();
        let mut groups = full.ledger.groups.clone();
        let mut clock = full.clock.clone();
        let mut adjustments: BTreeMap<TransactionId, Vec<Adjustment>> = BTreeMap::new();
        let inputs_now = |present: &BTreeMap<RecordId, (RecordInfo, Vec<Transaction>)>, accounts: &BTreeMap<AccountId, _>, instruments: &BTreeMap<InstrumentId, _>, trades: &bagholder_engine::input::Ranked<bagholder_core::journal::Trade>, groups: &bagholder_engine::input::Ranked<bagholder_core::journal::Group>, clock: &bagholder_engine::input::Clock, adjustments: &BTreeMap<TransactionId, Vec<Adjustment>>| -> Inputs {
            let mut i = full.clone();
            let txs: Vec<Transaction> = present.values().flat_map(|(_, t)| t.iter().cloned()).collect();
            let ids: BTreeSet<TransactionId> = txs.iter().map(|t| t.id.clone()).collect();
            i.ledger.transactions = txs.into_iter().collect();
            i.ledger.records = present.iter().map(|(r, (info, _))| (*r, info.clone())).collect();
            i.ledger.transfer_links = all_links.iter().filter(|(o, n)| ids.contains(o) && ids.contains(n)).cloned().collect();
            i.ledger.accounts = accounts.clone();
            i.ledger.instruments = instruments.clone();
            i.ledger.trades = trades.clone();
            i.ledger.groups = groups.clone();
            i.clock = clock.clone();
            i.facts.adjustments = Adjustments::choose(adjustments.values().flatten().filter(|a| ids.contains(&a.applies_to)).cloned());
            i
        };
        let mut e = Engine::build(inputs_now(&present, &accounts, &instruments, &trades, &groups, &clock, &adjustments));
        bagholder_engine::engine::take_paths();
        let mut rank = 100usize;
        for (n, step) in steps().into_iter().enumerate() {
            let before_fresh = Engine::build(e.inputs().clone());
            let mut d = LedgerDelta::default();
            let mut change = None;
            for l in &step.records {
                let r = record_of(&b, l);
                present.insert(r, whole[&r].clone());
                d.records.insert(r, Some(whole[&r].clone()));
            }
            match step.extra {
                Extra::None => {}
                Extra::NewAccount => {
                    let a = b.ids.account("N");
                    accounts.insert(a, full.ledger.accounts[&a].clone());
                    d.accounts.insert(a, Some(full.ledger.accounts[&a].clone()));
                }
                Extra::Terms => {
                    let o = b.ids.instrument("O");
                    let mut info = instruments[&o].clone();
                    if let Some(t) = info.terms.as_mut() {
                        t.multiplier = Some(bagholder_core::Dec::from_int(10));
                    }
                    instruments.insert(o, info.clone());
                    d.instruments.insert(o, Some(info));
                }
                Extra::CoinMinimum => {
                    let c = b.ids.account("C");
                    let mut info = accounts[&c].clone();
                    info.coin_minimum = Some(bagholder_core::Money::new(bagholder_core::Dec::from_int(5), bagholder_core::Currency::CAD));
                    accounts.insert(c, info.clone());
                    d.accounts.insert(c, Some(info));
                }
                Extra::NewContract => {
                    let q = b.ids.instrument("Q");
                    instruments.insert(q, full.ledger.instruments[&q].clone());
                    d.instruments.insert(q, Some(full.ledger.instruments[&q].clone()));
                }
                Extra::TradeOn(fill) => {
                    let id = b.ids.trade(&format!("on-{fill}"));
                    rank += 1;
                    let t = bagholder_core::journal::Trade { id, anchor: bagholder_core::journal::Anchor::Opening(bagholder_core::journal::Opening { transaction: b.tx[fill].clone(), instrument: b.ids.instrument(step.on) }), legacy_key: None };
                    trades.put(common::rank(rank), t.clone());
                    d.trades.insert(id, Some((common::rank(rank), t)));
                }
                Extra::TradeGone => {
                    let id = b.ids.trade("ts");
                    trades.remove(&id);
                    d.trades.insert(id, None);
                }
                Extra::Group => {
                    let id = b.ids.group("g");
                    rank += 1;
                    let g = bagholder_core::journal::Group { id, locked: true, members: vec![b.ids.trade("ts"), b.ids.trade("tt")] };
                    groups.put(common::rank(rank), g.clone());
                    d.groups.insert(id, Some((common::rank(rank), g)));
                }
                Extra::Clock => {
                    clock.today = clock.today.tomorrow().unwrap();
                    change = Some(Change::Clock(clock.clone()));
                }
                Extra::Split | Extra::CostFromT => {
                    let (row, leg) = match step.extra {
                        Extra::Split => ("e1", bagholder_engine::input::AdjustmentLeg { from: Some(b.ids.instrument("S")), to: Some(b.ids.instrument("S")), units_per_unit: Some(bagholder_core::Dec::from_int(2)), ..Default::default() }),
                        _ => ("c1", bagholder_engine::input::AdjustmentLeg { from: Some(b.ids.instrument("T")), to: Some(b.ids.instrument("S")), cost: Some(bagholder_core::Money::new(bagholder_core::Dec::from_int(50), bagholder_core::Currency::CAD)), ..Default::default() }),
                    };
                    let tx = b.tx[row].clone();
                    let a = vec![Adjustment { applies_to: tx.clone(), legs: vec![leg], source: bagholder_core::SourceName::person() }];
                    adjustments.insert(tx.clone(), a.clone());
                    d.adjustments.insert(tx, a);
                }
                Extra::Revise(label) => {
                    let r = record_of(&b, label);
                    let (info, mut txs) = present[&r].clone();
                    for t in &mut txs {
                        t.quantity = Some(bagholder_core::Dec::from_int(-1));
                    }
                    present.insert(r, (info.clone(), txs.clone()));
                    d.records.insert(r, Some((info, txs)));
                }
                Extra::Remove(label) => {
                    let r = record_of(&b, label);
                    present.remove(&r);
                    d.records.insert(r, None);
                }
                Extra::AdjustNamingT => {
                    let tx = b.tx["a5"].clone();
                    let leg = bagholder_engine::input::AdjustmentLeg { from: Some(b.ids.instrument("T")), to: Some(b.ids.instrument("T")), units_per_unit: Some(bagholder_core::Dec::from_int(1)), ..Default::default() };
                    let a = vec![Adjustment { applies_to: tx.clone(), legs: vec![leg], source: bagholder_core::SourceName::person() }];
                    adjustments.insert(tx.clone(), a.clone());
                    d.adjustments.insert(tx, a);
                }
                Extra::Unstated => {
                    let r = record_of(&b, "w1");
                    let (mut info, txs) = present[&r].clone();
                    info.problems.push(bagholder_core::record::Problem::new("leg-unstated", "stated by the test"));
                    present.insert(r, (info.clone(), txs.clone()));
                    d.records.insert(r, Some((info, txs)));
                }
            }
            let now = inputs_now(&present, &accounts, &instruments, &trades, &groups, &clock, &adjustments);
            let ids: BTreeSet<RecordId> = d.records.keys().copied().collect();
            d.links = now.ledger.transfer_links.iter().filter(|(o, i)| ids.contains(&o.record) || ids.contains(&i.record)).map(|(o, i)| (o.clone(), i.clone())).collect();
            // only the engine's own apply counts: the builds beside it match whole
            bagholder_engine::engine::take_paths();
            let moved = e.apply(change.unwrap_or(Change::Ledger(d)));
            let paths = bagholder_engine::engine::take_paths();
            let family = e.family_of(Node::Instrument(b.ids.instrument(step.on)));
            let took: Vec<bool> = paths.iter().filter(|(f, _)| *f == family).map(|(_, c)| *c).collect();
            let label = format!("seed {seed}, step {n} ({:?}, {:?})", step.records, step.path);
            match step.path {
                Path::Continued => {
                    assert_eq!(took, vec![true], "{label}: the family's match is continued, and only that");
                    continued_steps += 1;
                }
                Path::Whole(why) => assert!(!took.is_empty() && took.iter().all(|c| !c), "{label}: matched whole ({why}), took {took:?}"),
                Path::None => assert!(took.is_empty(), "{label}: nothing matched, took {took:?}"),
            }
            let fresh = Engine::build(now.clone());
            let off = e.differences(&fresh);
            assert!(off.is_empty(), "{label}: the figures differ from a fresh build in {:?}", off.0);
            if let Some(why) = e.match_disagrees(&fresh) {
                panic!("{label}: {why}");
            }
            assert!(e.figures().matched == &match_lots(&now), "{label}: the families' match is not the whole ledger's");
            assert_eq!(moved, fresh.differences(&before_fresh), "{label}: what moved is not what differs");
            let _ = &mut b;
        }
    }
    assert!(continued_steps >= 30 * 10, "only {continued_steps} steps were continued");
}

/// What a coin's sale leaves is dust, or not, at the rate of the row's own day:
/// while that rate is not known it is kept, and when it arrives it is dust, as a
/// fresh build has it (#513).
/// A later row then continues the family's match from the state the rate left.
#[test]
fn a_coin_judged_while_its_rate_is_pending_is_judged_again_when_the_rate_arrives() {
    let case = |rate: bool| {
        let mut c = json!({
            "today": "2026-12-31",
            "accounts": [{"id": "C", "coin_minimum": "1.00 CAD"}],
            "instruments": [{"id": "KU", "kind": "crypto", "currency": "USD"}],
            "transactions": [
                {"id": "b", "account": "C", "day": "2026-05-04", "at": "2026-05-04T15:00:00Z", "kind": "buy", "instrument": "KU", "qty": "1.000100", "cash": "-2.0002"},
                {"id": "s", "account": "C", "day": "2026-05-05", "at": "2026-05-05T15:00:00Z", "kind": "sell", "instrument": "KU", "qty": "-1.000000", "cash": "2.000000"},
                {"id": "later", "account": "C", "day": "2026-06-01", "at": "2026-06-01T15:00:00Z", "kind": "buy", "instrument": "KU", "qty": "0.500000", "cash": "-1"}
            ],
        });
        if rate {
            c["rates"] = json!({"USD": {"2026-05-05": "1.3700", "2026-06-01": "1.3600"}});
            c["covered"] = json!({"USD": [["2026-01-01", "2026-12-31"]]});
        }
        c
    };
    let (pending, mut known) = (build(&case(false)), build(&case(true)));
    let later = known.tx["later"].record;
    let without = |i: &Inputs| {
        let mut i = i.clone();
        i.ledger.transactions = i.ledger.transactions.iter().filter(|t| t.id.record != later).cloned().collect();
        i.ledger.records.remove(&later);
        i
    };
    let mut e = Engine::build(without(&pending.inputs));
    assert!(e.figures().matched.dust.is_empty(), "kept while the rate is pending");
    e.apply(Change::Rates(known.inputs.facts.rates.clone()));
    let fresh = Engine::build(without(&known.inputs));
    assert!(!fresh.figures().matched.dust.is_empty(), "dust once the rate is known");
    assert!(e.differences(&fresh).is_empty() && e.match_disagrees(&fresh).is_none(), "the engine as a fresh build once the rate arrives");
    // a later row continues from what the rate left
    let mut d = LedgerDelta::default();
    d.records.insert(later, Some((known.inputs.ledger.records[&later].clone(), known.inputs.ledger.transactions.of_record(later).cloned().collect())));
    bagholder_engine::engine::take_paths();
    e.apply(Change::Ledger(d));
    let family = e.family_of(Node::Instrument(known.ids.instrument("KU")));
    assert_eq!(bagholder_engine::engine::take_paths(), vec![(family, true)]);
    let fresh = Engine::build(known.inputs.clone());
    assert!(e.differences(&fresh).is_empty() && e.match_disagrees(&fresh).is_none(), "the continued match as a fresh build");
}
