//! What each fill did to its position, as the match records it
//! (`Matched::roles`): the executions table's words come from it (SPEC.md §Trades,
//! trade detail: an option's `BUY TO OPEN`, `SELL TO CLOSE`; a fill that closed one
//! trade and opened the next, `(close + open)`).

mod common;

use serde_json::json;

use bagholder_engine::ledger::FillRole;
use common::*;

#[test]
fn each_fill_says_whether_it_opened_closed_or_both() {
    let case = json!({
        "today": "2026-06-01",
        "accounts": [{"id": "A", "kind": "margin"}],
        "instruments": [
            {"id": "U", "currency": "USD", "symbol": "UUU"},
            {"id": "C1", "kind": "option", "currency": "USD", "symbol": "UUU 19JUN26 10.00 CALL", "underlying": "U", "expiry": "2026-06-19", "strike": "10", "right": "call", "multiplier": "100"}
        ],
        "rates": {"USD": {"2026-01-02": "1.35"}},
        "covered": {"USD": [["2026-01-01", "2026-12-31"]]},
        "transactions": [
            // a short of two opened, one covered, then a buy of three: covers the last and opens a long of two
            {"id": "open", "account": "A", "day": "2026-03-02", "kind": "sell", "instrument": "C1", "qty": "-2", "cash": "300"},
            {"id": "part", "account": "A", "day": "2026-03-09", "kind": "buy", "instrument": "C1", "qty": "1", "cash": "-50"},
            {"id": "through", "account": "A", "day": "2026-04-01", "kind": "buy", "instrument": "C1", "qty": "3", "cash": "-150"},
            {"id": "close", "account": "A", "day": "2026-04-08", "kind": "sell", "instrument": "C1", "qty": "-2", "cash": "120"},
            // shares: bought, then sold in part
            {"id": "buy", "account": "A", "day": "2026-03-02", "kind": "buy", "instrument": "U", "qty": "10", "cash": "-100"},
            {"id": "sell", "account": "A", "day": "2026-03-03", "kind": "sell", "instrument": "U", "qty": "-4", "cash": "44"}
        ]
    });
    let mut b = build(&case);
    let e = engine(&mut b);
    let role = |label: &str| e.figures().matched.roles.get(&b.tx[label]).copied();
    assert_eq!(role("open"), Some(FillRole { closed: false, opened: true }));
    assert_eq!(role("part"), Some(FillRole { closed: true, opened: false }));
    assert_eq!(role("through"), Some(FillRole { closed: true, opened: true }));
    assert_eq!(role("close"), Some(FillRole { closed: true, opened: false }));
    assert_eq!(role("buy"), Some(FillRole { closed: false, opened: true }));
    assert_eq!(role("sell"), Some(FillRole { closed: true, opened: false }));
}
