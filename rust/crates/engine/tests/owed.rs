//! The distributions an account is owed and has not been paid yet
//! (`docs/plans/distributions-owed.md`, `SPEC.md` `cashflow-40`): the cases in
//! `fixtures/owed_cases.json`, their expectations written from the rule by an
//! agent that had not read the engine, judged from Wealthsimple's positions read
//! as of the day before each ex-date, never from the journal's lots.

mod common;

use bagholder_engine::Change;
use bagholder_engine::scope::Filters;
use common::*;
use serde_json::Value;

#[test]
fn an_account_is_owed_what_it_held_into_the_ex_date_until_the_payment_posts() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/owed_cases.json")).unwrap();
    let mut failed = Vec::new();
    for case in &cases {
        let name = s(case, "name").unwrap();
        let mut b = build(&case["book"]);
        let e = engine(&mut b);
        let f = Filters::default();
        let owed = e.cashflow(&f, &e.portfolio(&f)).owed;
        let mut want_rows = 0;
        for want in arr(case, "expect") {
            if let Some(w) = s(&want, "waits_on") {
                if !e.needs().holdings_on.contains(&day(w)) {
                    failed.push(format!("{name}: {w} is not among the days asked for"));
                }
                continue;
            }
            want_rows += 1;
            let (i, a, ex) = (b.ids.instrument(s(&want, "instrument").unwrap()), b.ids.account(s(&want, "account").unwrap()), day(s(&want, "ex").unwrap()));
            match owed.iter().find(|o| o.instrument == i && o.account == a && o.ex_date == ex) {
                None => failed.push(format!("{name}: no row owed for {want}; owed {owed:?}")),
                Some(o) => {
                    let amount = o.amount.as_ref().map(|m| m.amount);
                    if o.qty != dec(s(&want, "qty").unwrap()) || o.per.amount != dec(s(&want, "per").unwrap()) || amount != Ok(dec(s(&want, "amount").unwrap())) {
                        failed.push(format!("{name}: owed {} × {} = {amount:?}, expected {want}", o.qty, o.per.amount));
                    }
                }
            }
        }
        if owed.len() != want_rows {
            failed.push(format!("{name}: {} rows owed, expected {want_rows}: {owed:?}", owed.len()));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// A distribution's form, where the payer's record does not state it, waits on
/// whether the account held the payer into the ex-date as the day's positions
/// state it (`cashflow::held_into`, the one definition the owed rows use), and is
/// judged again the moment that day is read: by the change as by a fresh build.
#[test]
fn a_distribution_s_form_waits_on_the_day_s_positions_and_is_judged_when_they_are_read() {
    let book: Value = serde_json::json!({
        "today": "2026-05-20",
        "accounts": [{"id": "A"}],
        "instruments": [{"id": "F", "symbol": "ZZFU"}],
        "frequencies": {"F": 12},
        "declared": {"F": [
            {"ex": "2026-03-13", "pay": "2026-03-20", "amount": "0.10", "form": "stated"},
            {"ex": "2026-04-15", "pay": "2026-04-22", "amount": "0.11", "form": "unstated"}
        ]},
        "transactions": [
            {"id": "t1", "account": "A", "day": "2026-01-02", "at": "2026-01-02T15:00:00Z", "kind": "buy", "instrument": "F", "qty": "100", "cash": "-1000"},
            {"id": "d1", "account": "A", "day": "2026-03-20", "at": "2026-03-20T11:00:00Z", "kind": "dividend", "instrument": "F", "cash": "10", "cash_currency": "CAD"}
        ],
        "brokers": {"A": {"held": {}, "days_held": {"2026-03-12": {"held": {"F": "100"}}}}}
    });
    let mut b = build(&book);
    let mut e = engine(&mut b);
    let (i, a) = (b.ids.instrument("F"), b.ids.account("A"));
    let waits = day("2026-04-14");
    // no dividend posted two business days after the pay date: units, if the
    // account held it into the ex-date, which that day's read has not said yet
    let per = e.figures().payers[&i].per.clone();
    assert!(per.as_ref().err().is_some_and(|g| g.iter().any(|x| matches!(x, bagholder_engine::gap::Gap::HoldingsUnread(d) if *d == waits))), "{per:?}");
    assert!(e.needs().holdings_on.contains(&waits));
    // the day read: held, so paid in units and passed over; the rate is the one before
    let mut stated = b.inputs.market.brokers[&a].clone();
    stated.days_held.insert(waits, bagholder_engine::input::HeldDay { held: [(i, dec("100"))].into(), ..Default::default() });
    e.apply(Change::Broker(a, Some(stated.clone())));
    assert_eq!(e.figures().payers[&i].per.as_ref().map(|m| m.amount), Ok(dec("0.10")), "judged when the day is read");
    b.inputs.market.brokers.insert(a, stated);
    let fresh = engine(&mut b);
    assert_eq!(e.figures().payers[&i], fresh.figures().payers[&i], "the change as a fresh build");
}

/// A Wealthsimple account whose past days have never been read, holding the payer
/// in the journal, is the broker's to read: no row stands on the journal's units,
/// and the day before the ex-date is asked for (`SPEC.md` `cashflow-41`, `-44`).
#[test]
fn an_account_the_broker_states_waits_on_its_day_and_is_never_judged_by_the_journal() {
    let book: Value = serde_json::json!({
        "today": "2026-04-17",
        "accounts": [{"id": "A"}],
        "instruments": [{"id": "F", "symbol": "ZZFU"}],
        "frequencies": {"F": 12},
        "declared": {"F": [{"ex": "2026-04-15", "pay": "2026-04-22", "amount": "0.125", "form": "stated"}]},
        "transactions": [
            {"id": "t1", "account": "A", "day": "2026-01-02", "at": "2026-01-02T15:00:00Z", "kind": "buy", "instrument": "F", "qty": "100", "cash": "-1000"}
        ],
        "brokers": {"A": {"held": {"F": "100"}, "net_value": {"2026-01-02": "1000", "2026-04-16": "1010"}, "now": "1010"}}
    });
    let mut b = build(&book);
    let e = engine(&mut b);
    let f = Filters::default();
    assert!(e.cashflow(&f, &e.portfolio(&f)).owed.is_empty(), "no row on the journal's units");
    assert!(e.needs().holdings_on.contains(&day("2026-04-14")), "the day before the ex-date asked for");
}
