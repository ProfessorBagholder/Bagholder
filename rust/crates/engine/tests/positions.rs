//! A covered call is a leg of its shares' position (SPEC.md trade-76, trade-77;
//! `docs/plans/covered-call-positions.md`): the cases in
//! `fixtures/position_cases.json`, their expectations written from the rule by an
//! agent that had not read the engine. The position's P&L, the calls written
//! against nothing, and every closed trade as the statistics count it.

mod common;

use std::collections::BTreeMap;

use bagholder_core::{Dec, TransactionId};
use bagholder_engine::coverage::{counted, positions, Coverage};
use bagholder_engine::trades::{TradeFig, TradeKey};
use common::*;
use serde_json::Value;

fn opening(t: &TradeFig) -> &TransactionId {
    match &t.key {
        TradeKey::Trip(k) => &k.opening,
        TradeKey::Group(_) => panic!("no saved groups in these books"),
    }
}

fn cents(d: Dec) -> Dec {
    d.round(2, bagholder_core::Rounding::HalfEven)
}

#[test]
fn a_covered_call_counts_inside_its_shares_position() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/position_cases.json")).unwrap();
    let mut failed = Vec::new();
    for case in &cases {
        let name = s(case, "name").unwrap();
        let mut b = build(&case["book"]);
        let e = engine(&mut b);
        let label: BTreeMap<&TransactionId, &str> = b.tx.iter().map(|(k, v)| (v, k.as_str())).collect();
        let name_of = |t: &TradeFig| label.get(opening(t)).copied().unwrap_or("?").to_string();
        let trades = e.figures().trades;
        let expect = &case["expect"];
        // the positions: each shares' trade, its calls and the position's P&L
        let mut got: Vec<(String, Vec<String>, Option<Dec>)> = positions(trades, e.figures().positions)
            .values()
            .map(|c| {
                let mut calls: Vec<String> = c.calls.iter().map(|t| name_of(t)).collect();
                calls.sort();
                (name_of(c.trade), calls, c.pnl.as_ref().ok().map(|m| cents(m.amount)))
            })
            .collect();
        got.sort();
        let mut want: Vec<(String, Vec<String>, Option<Dec>)> = arr(expect, "positions")
            .iter()
            .map(|p| {
                let mut calls: Vec<String> = arr(p, "calls").iter().map(|c| c.as_str().unwrap().to_string()).collect();
                calls.sort();
                (s(p, "shares_opened_by").unwrap().to_string(), calls, Some(dec(s(p, "pnl").unwrap())))
            })
            .collect();
        want.sort();
        if got != want {
            failed.push(format!("{name}: positions {got:?}, expected {want:?}"));
        }
        // the covered calls written against nothing
        let mut nothing: Vec<String> = trades.iter().filter(|t| matches!(&t.coverage, Some(Coverage::Read { covered, against: None, .. }) if covered.is_positive())).map(|t| name_of(t)).collect();
        nothing.sort();
        let mut want_nothing: Vec<String> = arr(expect, "linked_to_nothing").iter().map(|c| c.as_str().unwrap().to_string()).collect();
        want_nothing.sort();
        if nothing != want_nothing {
            failed.push(format!("{name}: written against nothing {nothing:?}, expected {want_nothing:?}"));
        }
        // every closed trade as the statistics count it, and its wins and losses
        let closed: Vec<&TradeFig> = trades.iter().filter(|t| t.is_closed()).collect();
        let c = counted(&closed, trades);
        let mut got_counted: Vec<(String, Option<Dec>)> = c.iter().map(|x| (name_of(x.trade), x.pnl.as_ref().ok().map(|m| cents(m.amount)))).collect();
        got_counted.sort();
        let mut want_counted: Vec<(String, Option<Dec>)> = arr(expect, "counted").iter().map(|x| (s(x, "opened_by").unwrap().to_string(), Some(dec(s(x, "pnl").unwrap())))).collect();
        want_counted.sort();
        if got_counted != want_counted {
            failed.push(format!("{name}: counted {got_counted:?}, expected {want_counted:?}"));
        }
        let wins = c.iter().filter(|x| x.pnl.as_ref().is_ok_and(|m| m.amount.is_positive())).count();
        let losses = c.iter().filter(|x| x.pnl.as_ref().is_ok_and(|m| m.amount.is_negative())).count();
        let (w, l) = (expect["wins"].as_u64().unwrap() as usize, expect["losses"].as_u64().unwrap() as usize);
        if (wins, losses) != (w, l) {
            failed.push(format!("{name}: {wins} wins and {losses} losses, expected {w} and {l}"));
        }
        // and the Dashboard reads the same
        let d = e.dashboard(&Default::default());
        if (d.kpi.wins, d.kpi.losses) != (w, l) {
            failed.push(format!("{name}: the Dashboard says {} wins and {} losses, expected {w} and {l}", d.kpi.wins, d.kpi.losses));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// A book: shares of U bought, a call written against them on 2026-03-03 (the
/// day's read states the shares), bought back on 2026-04-01, and the shares sold
/// on `sold`.
fn written_against_a_sale(sold: &str) -> Value {
    serde_json::json!({
        "today": "2026-06-01",
        "accounts": [{"id": "A", "kind": "margin"}],
        "instruments": [
            {"id": "U", "currency": "USD", "symbol": "UUU"},
            {"id": "C1", "kind": "option", "currency": "USD", "symbol": "UUU 19JUN26 10.00 CALL", "underlying": "U", "expiry": "2026-06-19", "strike": "10", "right": "call", "multiplier": "100"}
        ],
        "rates": {"USD": {"2026-01-02": "1.35"}},
        "covered": {"USD": [["2026-01-01", "2026-12-31"]]},
        "transactions": [
            {"id": "b1", "account": "A", "day": "2026-03-01", "at": "2026-03-01T15:00:00Z", "kind": "buy", "instrument": "U", "qty": "100", "cash": "-700"},
            {"id": "t1", "account": "A", "day": "2026-03-03", "at": "2026-03-03T15:00:00Z", "kind": "sell", "instrument": "C1", "qty": "-1", "cash": "40"},
            {"id": "t2", "account": "A", "day": "2026-04-01", "at": "2026-04-01T15:00:00Z", "kind": "buy", "instrument": "C1", "qty": "1", "cash": "-90"},
            {"id": "s1", "account": "A", "day": sold, "at": format!("{sold}T15:00:00Z"), "kind": "sell", "instrument": "U", "qty": "-100", "cash": "900"}
        ],
        "brokers": {"A": {"held": {}, "days_held": {"2026-03-03": {"held": {"U": "100", "C1": "-1"}, "cash": {"USD": "1000"}}}}}
    })
}

/// A sale moved so the shares' trade no longer covers the day the call was
/// written: the call's shares, judged again step by step, are a fresh build's.
#[test]
fn a_shares_sale_moved_before_the_call_leaves_it_written_against_nothing_as_a_fresh_build() {
    let mut before = build(&written_against_a_sale("2026-04-20"));
    let mut after = build(&written_against_a_sale("2026-03-02"));
    let mut e = engine(&mut before);
    let call = |e: &bagholder_engine::Engine| e.figures().trades.iter().find(|t| matches!(&t.coverage, Some(Coverage::Read { .. }))).and_then(|t| match &t.coverage {
        Some(Coverage::Read { against, .. }) => Some(against.is_some()),
        _ => None,
    });
    assert_eq!(call(&e), Some(true), "written against the shares held that day");
    let sale = after.inputs.ledger.transactions.iter().find(|t| t.id == after.tx["s1"]).unwrap().clone();
    let record = sale.id.record;
    let info = after.inputs.ledger.records.get(&record).cloned().unwrap();
    let moved = bagholder_engine::input::LedgerDelta { records: BTreeMap::from([(record, Some((info, vec![sale])))]), ..Default::default() };
    e.apply(bagholder_engine::Change::Ledger(moved));
    let fresh = engine(&mut after);
    assert_eq!(call(&fresh), Some(false), "the shares were sold before the call was written");
    assert_eq!(call(&e), Some(false), "judged again when the shares' trade moved");
    let off = e.differences(&fresh);
    assert!(off.is_empty(), "the figures differ from a fresh build in {:?}", off.0);
}

/// The shares' trade saved in a group, nothing else moving: the call's shares are
/// the group's trade now, judged again as a fresh build judges them.
#[test]
fn a_shares_trade_put_in_a_group_is_what_its_calls_are_written_against_as_a_fresh_build() {
    let mut b = build(&written_against_a_sale("2026-04-20"));
    let mut e = engine(&mut b);
    let shares = e.figures().trades.iter().find(|t| t.kind == bagholder_core::instrument::InstrumentKind::Security).and_then(|t| t.trade).expect("the shares' trade has an id");
    let g = b.ids.group("G");
    let group = bagholder_engine::input::LedgerDelta { groups: BTreeMap::from([(g, Some((rank(1000), bagholder_core::journal::Group { id: g, locked: true, members: vec![shares] })))]), ..Default::default() };
    let mut fresh_inputs = e.inputs().clone();
    e.apply(bagholder_engine::Change::Ledger(group.clone()));
    fresh_inputs.ledger.groups.put(rank(1000), bagholder_core::journal::Group { id: g, locked: true, members: vec![shares] });
    let fresh = bagholder_engine::Engine::build(fresh_inputs);
    let against = |e: &bagholder_engine::Engine| e.figures().trades.iter().find_map(|t| match &t.coverage {
        Some(Coverage::Read { against, .. }) => Some(against.clone()),
        _ => None,
    });
    assert_eq!(against(&fresh), Some(Some(TradeKey::Group(g))), "written against the group's trade");
    assert_eq!(against(&e), against(&fresh), "judged again when the shares' trade moved alone");
    let off = e.differences(&fresh);
    assert!(off.is_empty(), "the figures differ from a fresh build in {:?}", off.0);
}

