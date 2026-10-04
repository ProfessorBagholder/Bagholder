//! What a row not yet final holds against its account
//! (`docs/plans/broker-check-reserved-cash.md`, `SPEC.md` the header's broker
//! check): the owner's recorded pending orders (a limit buy, a stock sale, a call
//! sale, anonymised), each documented kind varied from them, and every status.

mod common;

use bagholder_core::hold::HoldKind;
use bagholder_core::json::{self, Value};
use bagholder_core::{Currency, Dec};
use common::*;

fn recorded(key: &str) -> Value {
    let text = std::fs::read_to_string(fixtures().parent().unwrap().join("wealthsimple-pending/pending-orders.json")).unwrap();
    let Value::Object(all) = json::parse(&text).unwrap() else { panic!("an object") };
    all[key].clone()
}

/// `v` with the row's fields set.
fn with(v: &Value, fields: &[(&str, Value)]) -> Value {
    let mut v = v.clone();
    let Value::Object(root) = &mut v else { panic!() };
    let Some(Value::Object(row)) = root.get_mut("activity") else { panic!() };
    for (k, x) in fields {
        row.insert(k.to_string(), x.clone());
    }
    v
}

fn text(s: &str) -> Value {
    Value::String(s.into())
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

#[test]
fn a_limit_buy_working_in_full_holds_the_amount_its_row_states() {
    let m = map_payload(&recorded("limit-buy"));
    let h = m.hold.expect("a hold");
    assert_eq!((h.kind, h.currency, h.amount), (HoldKind::Buy, Some(Currency::USD), Some(dec("70.00"))));
    assert!(m.legs.is_empty() && m.problems.is_empty());
}

#[test]
fn a_sale_holds_nothing() {
    for key in ["stock-sell", "call-sale"] {
        let m = map_payload(&recorded(key));
        assert_eq!(m.hold, None, "{key}");
        assert!(m.problems.is_empty(), "{key}: {:?}", m.problems);
    }
}

#[test]
fn any_other_buy_holds_an_amount_it_does_not_state() {
    let buy = recorded("limit-buy");
    let partly = with(&buy, &[("unifiedStatus", text("PARTIALLY_FILLED"))]);
    let mut rows = vec![partly];
    for sub in ["MARKET_ORDER", "FRACTIONAL_ORDER", "STOP_ORDER", "DIVIDEND_REINVESTMENT", "AN_ORDER_NO_READER_KNOWS"] {
        rows.push(with(&buy, &[("subType", text(sub))]));
    }
    for ty in ["DIY_BUY", "CRYPTO_BUY", "MANAGED_BUY", "PREDICTIONS_BUY"] {
        rows.push(with(&buy, &[("type", text(ty)), ("subType", text("MARKET_ORDER")), ("amount", Value::Null)]));
    }
    for r in rows {
        let h = map_payload(&r).hold.unwrap_or_else(|| panic!("no hold: {r:?}"));
        assert_eq!((h.kind, h.amount), (HoldKind::Buy, None), "{r:?}");
    }
}

#[test]
fn a_put_sold_states_its_collateral_and_premium() {
    // the recorded call sale, as a put: 40 contracts at a $3.00 strike on 100 units
    let call = recorded("call-sale");
    let Value::Object(root) = &call else { panic!() };
    let Value::Object(row) = &root["activity"] else { panic!() };
    let Value::String(id) = &row["securityId"] else { panic!() };
    let id = id.clone();
    let mut put = with(&call, &[("contractType", text("put"))]);
    let Value::Object(r) = &mut put else { panic!() };
    let Some(Value::Object(secs)) = r.get_mut("securities") else { panic!() };
    let Some(Value::Object(s)) = secs.get_mut(&id) else { panic!() };
    let Some(Value::Object(o)) = s.get_mut("optionDetails") else { panic!() };
    o.insert("optionType".into(), text("PUT"));
    let h = map_payload(&put).hold.expect("a hold");
    assert_eq!(h.kind, HoldKind::PutSale);
    assert_eq!((h.amount, h.quantity, h.premium), (Some(dec("12000")), Some(dec("40")), Some(dec("20000.00"))));
    assert!(h.instrument.is_some_and(|r| r.value == id));
}

#[test]
fn money_leaving_the_account_holds_an_amount_it_does_not_state() {
    let base = recorded("limit-buy");
    for (ty, sub) in [("WITHDRAWAL", "EFT"), ("WITHDRAWAL", "E_TRANSFER"), ("P2P_PAYMENT", "SEND"), ("INTERNAL_TRANSFER", "SOURCE"), ("INSTITUTIONAL_TRANSFER_INTENT", "TRANSFER_OUT")] {
        let r = with(&base, &[("type", text(ty)), ("subType", text(sub)), ("securityId", Value::Null)]);
        let h = map_payload(&r).hold.unwrap_or_else(|| panic!("no hold: {ty} {sub}"));
        assert_eq!((h.kind, h.amount), (HoldKind::Withdrawal, None), "{ty} {sub}");
    }
}

#[test]
fn a_final_row_holds_nothing_whatever_its_kind() {
    for key in ["limit-buy", "stock-sell", "call-sale"] {
        for status in ["COMPLETED", "CANCELLED", "DECLINED", "EXPIRED", "FAILED", "REJECTED", "REFUNDED", "REVERSED"] {
            let r = with(&recorded(key), &[("unifiedStatus", text(status))]);
            assert_eq!(map_payload(&r).hold, None, "{key} {status}");
        }
    }
}

#[test]
fn a_row_names_the_order_ids_it_carries() {
    let m = map_payload(&recorded("limit-buy"));
    assert_eq!(m.orders, vec!["order-00Yg9BqrHHNN".to_string(), "order-7526368f-c34d-4f6b-a3eb-bf9a007a5f08".to_string()]);
    // a row of no order names none
    let w = with(&recorded("limit-buy"), &[("type", text("WITHDRAWAL")), ("subType", text("EFT")), ("canonicalId", text("funding_intent-1")), ("externalCanonicalId", Value::Null), ("securityId", Value::Null)]);
    assert!(map_payload(&w).orders.is_empty());
}
