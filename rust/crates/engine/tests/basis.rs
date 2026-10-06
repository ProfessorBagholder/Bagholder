//! A holding's average cost (`SPEC.md` §2 Position, Book; brief 20 §2 and §5),
//! held over generated books: purchases and sales with commissions, shorts,
//! moves between the person's accounts, splits, stock dividends, returns of
//! capital, spin-offs, and option contracts written, bought, expired, assigned
//! and exercised.
//!
//! Every book is matched after each of its transactions, and each time:
//!
//! - the average cost is kept beside the lots and nowhere else: a side with no
//!   lots has none, and a side with lots has one, stated;
//! - the two methods account for the same money. On the long side, cost still
//!   open less what was realized moves only by what came in or went out (a
//!   purchase's cost, a sale's proceeds, capital returned), so it is the same
//!   under average cost and under the lots matched first in first out; on the
//!   short side the same holds of cost open plus what was realized. So whenever
//!   the book is flat, Σ realized at average cost equals Σ realized first in
//!   first out;
//! - a sale leaves the average cost of what is still held where it was.
//!
//! Transfers out of the person's accounts are left out: there the cost leaves
//! with no sale, the oldest lots' under one method and an average share under
//! the other, so the two are not meant to agree after one.

mod common;

use std::collections::BTreeMap;

use serde_json::{json, Value};

use bagholder_core::{Dec, Money, Rounding};
use bagholder_engine::ledger::{match_lots, Direction, Matched};
use common::*;

/// splitmix64: the books are the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// An amount with two places, from `lo` to `hi` whole units.
    fn cents(&mut self, lo: u64, hi: u64) -> String {
        let c = lo * 100 + self.below((hi - lo) * 100 + 1);
        format!("{}.{:02}", c / 100, c % 100)
    }
}

/// One generated book: its transactions, adjustments and links, as a case.
fn generate(seed: u64) -> Value {
    let mut r = Rng(seed);
    let mut tx: Vec<Value> = Vec::new();
    let mut adjustments: Vec<Value> = Vec::new();
    let mut links: Vec<Value> = Vec::new();
    // units of X each account holds, as the generator reckons it (an event may
    // make it differ; the identities hold either way)
    let mut held: BTreeMap<&str, i64> = BTreeMap::from([("A", 0), ("B", 0)]);
    let mut short_z: i64 = 0;
    let mut written: i64 = 0;
    let mut bought_put: i64 = 0;
    let steps = 6 + r.below(30);
    let mut day = 0u32;
    for n in 0..steps {
        day += 1 + r.below(9) as u32;
        let date = format!("2026-{:02}-{:02}", 1 + day / 28, 1 + day % 28);
        let at = |m: u32| format!("{date}T15:{m:02}:00Z");
        let id = format!("t{n}");
        let acct = if r.below(3) == 0 { "B" } else { "A" };
        let fee = if r.below(2) == 0 { Some(r.cents(0, 10)) } else { None };
        let with_fee = |mut v: Value, fee: &Option<String>| {
            if let Some(f) = fee {
                v["fee"] = json!(f);
            }
            v
        };
        match r.below(14) {
            // a purchase
            0..=3 => {
                let q = 1 + r.below(60) as i64;
                let price = r.cents(5, 80);
                let cost = Dec::parse(&price).unwrap().checked_mul(Dec::from_int(q)).unwrap();
                let fee_d = fee.as_deref().map(|f| Dec::parse(f).unwrap()).unwrap_or(Dec::ZERO);
                let cash = cost.checked_add(fee_d).unwrap().neg();
                tx.push(with_fee(json!({"id": id, "account": acct, "day": date, "at": at(0), "kind": "buy", "instrument": "X", "qty": q.to_string(), "cash": cash.to_text()}), &fee));
                *held.get_mut(acct).unwrap() += q;
            }
            // a sale of part or all of what is held
            4..=6 => {
                let h = held[acct];
                if h <= 0 {
                    continue;
                }
                let q = 1 + r.below(h as u64) as i64;
                let price = r.cents(5, 80);
                let value = Dec::parse(&price).unwrap().checked_mul(Dec::from_int(q)).unwrap();
                let fee_d = fee.as_deref().map(|f| Dec::parse(f).unwrap()).unwrap_or(Dec::ZERO);
                let cash = value.checked_sub(fee_d).unwrap();
                if cash.is_negative() {
                    continue;
                }
                tx.push(with_fee(json!({"id": id, "account": acct, "day": date, "at": at(0), "kind": "sell", "instrument": "X", "qty": (-q).to_string(), "cash": cash.to_text()}), &fee));
                *held.get_mut(acct).unwrap() -= q;
            }
            // a share sold short, or bought back
            7 => {
                if short_z > 0 && r.below(2) == 0 {
                    let q = 1 + r.below(short_z as u64) as i64;
                    let cash = format!("-{}", r.cents(100, 900));
                    tx.push(with_fee(json!({"id": id, "account": "A", "day": date, "at": at(0), "kind": "buy", "effect": "close", "instrument": "Z", "qty": q.to_string(), "cash": cash}), &fee));
                    short_z -= q;
                } else {
                    let q = 1 + r.below(40) as i64;
                    let cash = r.cents(100, 900);
                    tx.push(with_fee(json!({"id": id, "account": "A", "day": date, "at": at(0), "kind": "sell", "effect": "open", "instrument": "Z", "qty": (-q).to_string(), "cash": cash}), &fee));
                    short_z += q;
                }
            }
            // part or all of a holding moved to the other account
            8 => {
                let h = held[acct];
                if h <= 0 {
                    continue;
                }
                let q = 1 + r.below(h as u64) as i64;
                let to = if acct == "A" { "B" } else { "A" };
                let out = format!("{id}o");
                let inn = format!("{id}i");
                tx.push(json!({"id": out, "account": acct, "day": date, "at": at(0), "kind": "transfer-out", "instrument": "X", "qty": (-q).to_string()}));
                tx.push(json!({"id": inn, "account": to, "day": date, "at": at(1), "kind": "transfer-in", "instrument": "X", "qty": q.to_string()}));
                links.push(json!({"out": out, "in": inn}));
                *held.get_mut(acct).unwrap() -= q;
                *held.get_mut(to).unwrap() += q;
            }
            // a corporate event on X in an account that holds it and never holds
            // it short (an assignment can leave the other short, and what an
            // event other than a split does to a short is not worked out:
            // `SPEC.md` §2, the event waits)
            9 => {
                let acct = "B";
                if held[acct] <= 0 {
                    continue;
                }
                tx.push(json!({"id": id, "account": acct, "day": date, "at": at(0), "kind": "corporate-event", "instrument": "X"}));
                let leg = match r.below(4) {
                    0 => {
                        *held.get_mut(acct).unwrap() *= 2;
                        json!({"from": "X", "to": "X", "units_per_unit": "2"})
                    }
                    1 => json!({"from": "X", "to": "X", "cash_per_unit": r.cents(0, 30)}),
                    2 => {
                        let units = held[acct] / 10;
                        *held.get_mut(acct).unwrap() += units;
                        json!({"from": "X", "to": "X", "units_per_unit": "0.1", "cost": r.cents(1, 200)})
                    }
                    _ => json!({"from": "X", "to": "Y", "units_per_unit": "0.5", "cost_share": "0.2"}),
                };
                adjustments.push(json!({"applies_to": id, "legs": [leg]}));
            }
            // a call written, or bought back
            10 | 11 => {
                if written > 0 && r.below(2) == 0 {
                    tx.push(with_fee(json!({"id": id, "account": "A", "day": date, "at": at(0), "kind": "buy", "effect": "close", "instrument": "C", "qty": "1", "cash": format!("-{}", r.cents(10, 400))}), &fee));
                    written -= 1;
                } else {
                    tx.push(with_fee(json!({"id": id, "account": "A", "day": date, "at": at(0), "kind": "sell", "effect": "open", "instrument": "C", "qty": "-1", "cash": r.cents(10, 400)}), &fee));
                    written += 1;
                }
            }
            // a written call expires or is assigned; a put bought, exercised or expiring
            12 => {
                if written > 0 {
                    if r.below(2) == 0 {
                        tx.push(json!({"id": id, "account": "A", "day": date, "at": at(0), "kind": "option-expiry", "instrument": "C", "qty": "1"}));
                    } else {
                        tx.push(json!({"id": id, "account": "A", "day": date, "at": at(0), "kind": "option-assignment", "instrument": "C", "qty": "1", "cash": "5000"}));
                        *held.get_mut("A").unwrap() -= 100;
                    }
                    written -= 1;
                }
            }
            _ => {
                if bought_put > 0 {
                    if r.below(2) == 0 {
                        tx.push(json!({"id": id, "account": "A", "day": date, "at": at(0), "kind": "option-expiry", "instrument": "P", "qty": "-1"}));
                    } else {
                        tx.push(json!({"id": id, "account": "A", "day": date, "at": at(0), "kind": "option-exercise", "instrument": "P", "qty": "-1", "cash": "4000"}));
                        *held.get_mut("A").unwrap() -= 100;
                    }
                    bought_put -= 1;
                } else {
                    tx.push(with_fee(json!({"id": id, "account": "A", "day": date, "at": at(0), "kind": "buy", "effect": "open", "instrument": "P", "qty": "1", "cash": format!("-{}", r.cents(10, 300))}), &fee));
                    bought_put += 1;
                }
            }
        }
    }
    json!({
        "today": "2029-12-31",
        "accounts": [{"id": "A", "kind": "margin"}, {"id": "B"}],
        "instruments": [
            {"id": "X"}, {"id": "Y"}, {"id": "Z"},
            {"id": "C", "kind": "option", "underlying": "X", "expiry": "2030-06-21", "strike": "50", "right": "call", "multiplier": "100"},
            {"id": "P", "kind": "option", "underlying": "X", "expiry": "2030-06-21", "strike": "40", "right": "put", "multiplier": "100"}
        ],
        "transactions": tx,
        "adjustments": adjustments,
        "transfer_links": links,
    })
}

/// What one side of every holding sums to: the average cost open, what it
/// realized, the lots' own cost open (value with fee for a long, less it for a
/// short), and what the lots realized, matched first in first out.
#[derive(Default, Debug, PartialEq)]
struct Side {
    avg_open: Dec,
    avg_realized: Dec,
    lots_open: Dec,
    lots_realized: Dec,
}

fn add(a: &mut Dec, m: &Money) {
    *a = a.add_to_fit(m.amount).unwrap();
}

fn sides(seed: u64, upto: usize, m: &Matched) -> BTreeMap<Direction, Side> {
    let mut out: BTreeMap<Direction, Side> = BTreeMap::new();
    for ((a, i), book) in &m.books {
        for d in [Direction::Long, Direction::Short] {
            let s = out.entry(d).or_default();
            let lots: Vec<_> = book.lots.iter().filter(|l| l.direction == d).collect();
            match book.basis.get(&d) {
                Some(b) => {
                    let cost = b.cost.clone().unwrap_or_else(|g| panic!("book {seed}, after {upto}: {a} {i} {d:?} average cost waits on {g:?}"));
                    let realized = b.realized.clone().unwrap_or_else(|g| panic!("book {seed}, after {upto}: {a} {i} {d:?} realized waits on {g:?}"));
                    if lots.is_empty() {
                        assert!(cost.amount.is_zero(), "book {seed}, after {upto}: {a} {i} {d:?} holds nothing at a cost of {cost:?}");
                    }
                    add(&mut s.avg_open, &cost);
                    add(&mut s.avg_realized, &realized);
                }
                None => assert!(lots.is_empty(), "book {seed}, after {upto}: {a} {i} {d:?} holds lots with no average cost"),
            }
            for l in lots {
                let v = l.value.clone().unwrap_or_else(|g| panic!("book {seed}, after {upto}: a lot waits on {g:?}"));
                let c = match d {
                    Direction::Long => v.add_to_fit(l.fee).unwrap(),
                    Direction::Short => v.checked_sub(l.fee).unwrap(),
                };
                add(&mut s.lots_open, &c);
            }
        }
    }
    for t in m.trips.values() {
        for sl in &t.slices {
            let p = sl.pnl().unwrap_or_else(|g| panic!("book {seed}, after {upto}: a slice waits on {g:?}"));
            add(&mut out.entry(sl.direction).or_default().lots_realized, &p);
        }
    }
    out
}

#[test]
fn average_cost_and_the_lots_account_for_the_same_money_after_every_transaction() {
    let mut checked = 0;
    for seed in 1..=400u64 {
        let case = generate(seed);
        let full = build(&case);
        let n = full.inputs.ledger.transactions.len();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|k| (full.inputs.ledger.transactions[*k].trade_date, full.inputs.ledger.transactions[*k].occurred_at));
        let mut before: Option<Matched> = None;
        for upto in 1..=n {
            let mut inputs = full.inputs.clone();
            let keep: std::collections::BTreeSet<_> = order[..upto].iter().map(|k| full.inputs.ledger.transactions[*k].id.clone()).collect();
            inputs.ledger.transactions.retain(|t| keep.contains(&t.id));
            // a move between the accounts is one step: never cut between its sides
            if inputs.ledger.transfer_links.iter().any(|(o, i)| keep.contains(o) != keep.contains(i)) {
                continue;
            }
            let m = match_lots(&inputs);
            for (d, s) in sides(seed, upto, &m) {
                // long: open cost less realized; short: open cost plus realized
                let (avg, lots) = match d {
                    Direction::Long => (s.avg_open.checked_sub(s.avg_realized).unwrap(), s.lots_open.checked_sub(s.lots_realized).unwrap()),
                    Direction::Short => (s.avg_open.checked_add(s.avg_realized).unwrap(), s.lots_open.checked_add(s.lots_realized).unwrap()),
                };
                assert_eq!(avg, lots, "book {seed}, after {upto} transactions, {d:?}: {s:?}");
                if s.lots_open.is_zero() && s.avg_open.is_zero() {
                    assert_eq!(s.avg_realized, s.lots_realized, "book {seed}, flat after {upto}, {d:?}: realized at average cost against first in first out");
                }
            }
            // a sale leaves the average cost of what is still held where it was
            let last = &full.inputs.ledger.transactions[order[upto - 1]];
            if let (Some(prev), bagholder_core::transaction::Kind::Sell, None) = (&before, last.kind, last.effect) {
                let per = |m: &Matched| {
                    let b = m.books.get(&(last.account, last.instrument.unwrap()))?;
                    let units = b.lots.iter().filter(|l| l.direction == Direction::Long).try_fold(Dec::ZERO, |a, l| a.checked_add(l.qty)).ok()?;
                    let cost = b.basis.get(&Direction::Long)?.cost.clone().ok()?;
                    (units.is_positive()).then(|| cost.amount.div_rounded(units, 9, Rounding::HalfEven).unwrap())
                };
                if let (Some(a), Some(b)) = (per(prev), per(&m)) {
                    assert_eq!(a, b, "book {seed}: the sale {} moved the average cost", last.id);
                    checked += 1;
                }
            }
            before = Some(m);
        }
    }
    assert!(checked > 100, "only {checked} partial sales were checked");
    // every kind of transaction the books are meant to hold, many times over
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for seed in 1..=400u64 {
        let case = generate(seed);
        for t in case["transactions"].as_array().unwrap() {
            *kinds.entry(format!("{}{}", t["kind"].as_str().unwrap(), t.get("effect").map(|e| format!(" {}", e.as_str().unwrap())).unwrap_or_default())).or_default() += 1;
        }
        for a in case["adjustments"].as_array().unwrap() {
            let leg = &a["legs"][0];
            let kind = match (leg.get("cash_per_unit"), leg.get("cost"), leg["to"].as_str()) {
                (Some(_), _, _) => "return of capital",
                (_, Some(_), _) => "stock dividend",
                (_, _, Some("Y")) => "spin-off",
                _ => "split",
            };
            *kinds.entry(kind.into()).or_default() += 1;
        }
    }
    for k in ["buy", "sell", "sell open", "buy close", "buy open", "transfer-out", "option-expiry", "option-assignment", "option-exercise", "return of capital", "stock dividend", "spin-off", "split"] {
        assert!(kinds.get(k).copied().unwrap_or(0) >= 20, "{k}: only {:?} generated ({kinds:?})", kinds.get(k));
    }
}

