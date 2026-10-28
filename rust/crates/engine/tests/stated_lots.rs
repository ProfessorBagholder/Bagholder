//! Units the activity feed never showed arriving come from Wealthsimple's
//! positions read (brief 79, amended 2026-10-28, point 5; the owner: "but why is
//! that happening in the first place? tat should never happen"): where a sale
//! would take more than the journal holds, the read of the day before states the
//! units, a lot of them opens marked `stated`, and its cost is the read's book
//! value for the holding less the cost of the lots held, never below nothing. A
//! sale never takes more than was held.

mod common;

use serde_json::{json, Value};

use bagholder_core::Dec;
use bagholder_engine::ledger::{Flag, StatedRead};
use bagholder_engine::Change;
use common::*;

fn d(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

/// A book of one account holding `S`, its rows `transactions`, Wealthsimple
/// stating `days` of it, and nothing held now.
fn book(transactions: Value, days: Value) -> Value {
    json!({
        "today": "2026-03-02",
        "accounts": [{"id": "T", "kind": "cash"}],
        "instruments": [{"id": "S"}, {"id": "BTC", "kind": "crypto"}],
        "transactions": transactions,
        "brokers": {"T": {"held": {}, "days_held": days}}
    })
}

/// The realized P&L of every round trip closed, in the order they closed: none
/// where it waits on a figure.
fn realized(e: &bagholder_engine::Engine) -> Vec<Option<Dec>> {
    let mut trips: Vec<_> = e.figures().matched.trips.values().filter(|t| t.open_lots == 0 && !t.slices.is_empty()).collect();
    trips.sort_by_key(|t| t.slices.iter().map(|s| s.closed_on).max());
    trips.iter().map(|t| t.slices.iter().try_fold(Dec::ZERO, |a, s| s.pnl().ok().map(|p| a.checked_add(p.amount).unwrap()))).collect()
}

/// The stated lots opened, as their units and cost: none where it waits on a read.
fn stated(e: &bagholder_engine::Engine) -> Vec<(Dec, Option<Dec>)> {
    let mut out = Vec::new();
    for t in e.figures().matched.trips.values() {
        for s in &t.slices {
            if s.flags.contains(&Flag::Stated) {
                out.push((s.qty, s.entry.clone().ok().map(|m| m.amount)));
            }
        }
    }
    out
}

fn reads(e: &bagholder_engine::Engine) -> Vec<StatedRead> {
    e.figures().matched.beyond.iter().map(|b| b.read).collect()
}

/// Units transferred in that the feed never lists: the read states them, so the
/// lot carries the read's whole book value.
#[test]
fn units_moved_in_unlisted_carry_the_book_value_the_read_states() {
    let mut b = build(&book(
        json!([{"id": "t1", "account": "T", "day": "2026-02-10", "at": "2026-02-10T15:00:00Z", "kind": "sell", "instrument": "S", "qty": "-100", "cash": "1500"}]),
        json!({"2026-02-09": {"held": {"S": "100"}, "book": {"S": "1200.00 CAD"}}}),
    ));
    let e = engine(&mut b);
    assert_eq!(reads(&e), [StatedRead::Opened("2026-02-09".parse().unwrap())]);
    assert_eq!(stated(&e), [(d("100"), Some(d("1200")))]);
    assert_eq!(realized(&e), [Some(d("300"))]);
}

/// A split the feed leaves out: the units it brought cost nothing, the holding's
/// cost unchanged, and the sale's P&L is every unit's against what was paid.
#[test]
fn a_split_the_feed_omits_brings_its_units_at_no_cost() {
    let mut b = build(&book(
        json!([
            {"id": "t1", "account": "T", "day": "2026-01-05", "at": "2026-01-05T15:00:00Z", "kind": "buy", "instrument": "S", "qty": "100", "cash": "-1000"},
            {"id": "t2", "account": "T", "day": "2026-02-10", "at": "2026-02-10T15:00:00Z", "kind": "sell", "instrument": "S", "qty": "-200", "cash": "3000"}
        ]),
        json!({"2026-02-09": {"held": {"S": "200"}, "book": {"S": "1000.00 CAD"}}}),
    ));
    let e = engine(&mut b);
    assert_eq!(stated(&e), [(d("100"), Some(d("0")))], "the split's units at no cost, never a share of the book");
    assert_eq!(realized(&e), [Some(d("2000"))], "3,000 for what cost 1,000");
}

/// A coin's units the feed states short of what was held, past its rounding:
/// the units it left out cost what the read's book value is beyond the lots'.
#[test]
fn a_coins_units_the_feed_left_out_cost_what_the_read_states_beyond_the_lots() {
    let mut b = build(&book(
        json!([
            {"id": "t1", "account": "T", "day": "2026-01-05", "at": "2026-01-05T15:00:00Z", "kind": "buy", "instrument": "BTC", "qty": "1.000000", "cash": "-50000"},
            {"id": "t2", "account": "T", "day": "2026-02-10", "at": "2026-02-10T15:00:00Z", "kind": "sell", "instrument": "BTC", "qty": "-1.000200", "cash": "60012"}
        ]),
        json!({"2026-02-09": {"held": {"BTC": "1.0002"}, "book": {"BTC": "50010.00 CAD"}}}),
    ));
    let e = engine(&mut b);
    assert_eq!(stated(&e), [(d("0.0002"), Some(d("10")))]);
    assert!(!reads(&e).contains(&StatedRead::Unstated), "nothing taken beyond what was held");
}

/// A day not read yet: the sale waits on its read, the read is asked for, and
/// once it is read the sale is matched again with the units it states.
#[test]
fn a_day_not_read_yet_holds_the_sale_waiting_until_it_is_read() {
    let mut b = build(&book(
        json!([{"id": "t1", "account": "T", "day": "2026-02-10", "at": "2026-02-10T15:00:00Z", "kind": "sell", "instrument": "S", "qty": "-100", "cash": "1500"}]),
        json!({"2026-01-30": {"held": {}}}),
    ));
    let mut e = engine(&mut b);
    let day: bagholder_core::jiff::civil::Date = "2026-02-09".parse().unwrap();
    assert_eq!(reads(&e), [StatedRead::Unread(day)]);
    assert!(e.needs().before_sales.contains(&day), "the day is asked for");
    assert_eq!(stated(&e), [(d("100"), None)]);
    assert_eq!(realized(&e), [None], "a figure not read yet, never a guess");
    let a = b.ids.account("T");
    let mut stated_now = e.inputs().market.brokers[&a].clone();
    let s = b.ids.instrument("S");
    stated_now.days_held.insert(day, bagholder_engine::input::HeldDay { held: [(s, d("100"))].into(), book: [(s, bagholder_core::Money::new(d("1200"), bagholder_core::Currency::CAD))].into(), ..Default::default() });
    e.apply(Change::Broker(a, Some(stated_now)));
    assert_eq!(reads(&e), [StatedRead::Opened(day)]);
    assert_eq!(realized(&e), [Some(d("300"))]);
    assert!(!e.needs().before_sales.contains(&day));
}

/// A transfer in that states no value, where Wealthsimple's positions read of its
/// day states the units: they cost the read's book value, and nothing is asked
/// of the person (the opening-balance form only for an account no holdings are
/// read of).
#[test]
fn a_transfer_in_stating_no_value_costs_what_the_read_states_and_asks_nothing() {
    let mut b = build(&book(
        json!([
            {"id": "t1", "account": "T", "day": "2026-02-02", "at": "2026-02-02T15:00:00Z", "kind": "transfer-in", "instrument": "S", "qty": "100"},
            {"id": "t2", "account": "T", "day": "2026-02-10", "at": "2026-02-10T15:00:00Z", "kind": "sell", "instrument": "S", "qty": "-100", "cash": "1500"}
        ]),
        json!({"2026-02-02": {"held": {"S": "100"}, "book": {"S": "1200.00 CAD"}}, "2026-02-09": {"held": {"S": "100"}, "book": {"S": "1200.00 CAD"}}}),
    ));
    let e = engine(&mut b);
    assert!(e.figures().matched.waiting.is_empty(), "nothing asked of the person: {:?}", e.figures().matched.waiting);
    assert_eq!(stated(&e), [(d("100"), Some(d("1200")))]);
    assert_eq!(realized(&e), [Some(d("300"))]);
}
