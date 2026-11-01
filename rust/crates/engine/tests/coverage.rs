//! What a written option was held against when it was written
//! (`docs/plans/trade-side-and-coverage.md`): the cases in
//! `fixtures/coverage_cases.json`, their expectations written from the rule by an
//! agent that had not read the engine, judged from the positions read of each
//! contract's opening day, never from the journal's lots.

mod common;

use bagholder_core::instrument::OptionRight;
use bagholder_engine::coverage::Coverage;
use bagholder_engine::trades::TradeKey;
use common::*;
use serde_json::Value;

/// What the Side column reads for a trade's coverage (the server's `side_label`).
fn side(c: Option<&Coverage>) -> &'static str {
    match c {
        Some(c @ Coverage::Read { right, .. }) => match (right, c.whole()) {
            (OptionRight::Call, true) => "Covered call",
            (OptionRight::Call, false) => "Uncovered call",
            (OptionRight::Put, true) => "Secured put",
            (OptionRight::Put, false) => "Uncovered put",
        },
        _ => "Short",
    }
}

#[test]
fn a_written_contract_is_covered_by_what_the_day_s_read_states() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/coverage_cases.json")).unwrap();
    let mut failed = Vec::new();
    for case in &cases {
        let name = s(case, "name").unwrap();
        let mut b = build(&case["book"]);
        let e = engine(&mut b);
        for want in arr(case, "expect") {
            let by = &b.tx[s(&want, "opened_by").unwrap()];
            let Some(t) = e.figures().trades.iter().find(|t| matches!(&t.key, TradeKey::Trip(k) if &k.opening == by)) else {
                failed.push(format!("{name}: no trade opened by {}", s(&want, "opened_by").unwrap()));
                continue;
            };
            let c = t.coverage.as_ref();
            let got_side = side(c);
            if got_side != s(&want, "side").unwrap() {
                failed.push(format!("{name}: side {got_side:?}, expected {:?} ({c:?})", s(&want, "side").unwrap()));
            }
            match (c, s(&want, "covered"), s(&want, "waits_on")) {
                (Some(Coverage::Read { covered, of, .. }), Some(w), _) => {
                    if *covered != dec(w) || *of != dec(s(&want, "of").unwrap()) {
                        failed.push(format!("{name}: {covered} of {of}, expected {w} of {}", s(&want, "of").unwrap()));
                    }
                }
                (Some(Coverage::Waiting { day: Some(d), .. }), _, Some(w)) => {
                    if *d != day(w) {
                        failed.push(format!("{name}: waits on {d}, expected {w}"));
                    }
                    if !e.needs().holdings_on.contains(d) {
                        failed.push(format!("{name}: {d} is not among the days asked for"));
                    }
                }
                (Some(Coverage::Unclear { .. }), None, None) if want.get("untold").is_some() => {}
                (got, _, _) => failed.push(format!("{name}: coverage {got:?}, expected {want}")),
            }
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
