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
        let mut got: Vec<(String, Vec<String>, Option<Dec>)> = positions(trades)
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
