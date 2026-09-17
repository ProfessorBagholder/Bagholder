//! Option rows stored under Wealthsimple's labels, relabelled and rescaled by
//! `store.ensure` (tests/test_store.py).

mod common;
use bagholder_store::{relabel, tables};
use common::*;
use serde_json::{json, Value};
use std::collections::HashMap;

fn by_cid(d: &Db) -> HashMap<String, Value> {
    d.activities().into_iter().map(|a| (a["canonicalId"].as_str().unwrap_or("").to_string(), a)).collect()
}

fn opt(cid: &str, at: &str, day: &str, atype: &str, sub: &str, sym: &str, cur: &str, qty: f64, px: f64, cash: f64, cat: &str, raw: &str) -> Value {
    json!({"canonicalId": cid, "occurredAt": at, "transactionDate": day, "accountId": "acct-1", "accountType": "Trading",
           "activityType": atype, "activitySubType": sub, "symbol": sym, "currency": cur, "quantity": qty, "unitPrice": px,
           "netCashAmount": cash, "category": cat, "source": "wealthsimple", "rawType": raw})
}

#[test]
fn test_scale_stored_option_unit_price_missing_multiplier() {
    let d = db();
    d.apply(&[
        opt("opt-cheap-1", "2026-08-31T14:16:58Z", "2026-08-31", "OPTIONS_SELL", "SELLTOOPEN", "DRAM 19FEB27 1.00 CALL", "USD", -10.0, 11.25, 112.5, "trade", "OPTIONS_SELL"),
        opt("opt-ok-1", "2026-08-31T14:17:58Z", "2026-08-31", "OPTIONS_SELL", "SELLTOOPEN", "SOXL 19FEB27 20.00 CALL", "USD", -10.0, 13.3, 13300.0, "trade", "OPTIONS_SELL"),
        opt("share-ok-1", "2026-08-31T14:18:58Z", "2026-08-31", "Trade", "BUY", "AAA", "CAD", 10.0, 10.0, -100.0, "trade", "DIY_BUY"),
    ]);
    // setup already ran ensure() on an empty DB and stamped the one-shot key
    d.conn.execute("DELETE FROM meta WHERE key = ?", [relabel::OPTION_UNIT_PRICE_SCALE_META]).unwrap();
    d.ensure();
    let m = by_cid(&d);
    approx(f(&m["opt-cheap-1"]["unitPrice"]), 0.1125);
    approx(f(&m["opt-ok-1"]["unitPrice"]), 13.3);
    approx(f(&m["share-ok-1"]["unitPrice"]), 10.0);
    assert_eq!(tables::get_meta(&d.conn, relabel::OPTION_UNIT_PRICE_SCALE_META, "").unwrap(), "1");
    // stamped: a second ensure does not scale again
    d.ensure();
    let again = by_cid(&d);
    approx(f(&again["opt-cheap-1"]["unitPrice"]), 0.1125);
    approx(f(&again["opt-ok-1"]["unitPrice"]), 13.3);
    approx(f(&again["share-ok-1"]["unitPrice"]), 10.0);
}

#[test]
fn test_relabel_stored_options_sell() {
    let d = db();
    d.apply(&[opt("opt-sell-1", "2026-08-31T14:16:58Z", "2026-08-31", "OPTIONS_SELL", "LIMIT_ORDER", "QNC 19FEB27 3.00 CALL", "USD", 35.0, 0.3, 1050.0, "other", "OPTIONS_SELL")]);
    d.ensure();
    let row = by_cid(&d)["opt-sell-1"].clone();
    assert_eq!(row["activitySubType"], "SELLTOOPEN");
    assert_eq!(row["category"], "trade");
    assert_eq!(f(&row["quantity"]), -35.0);
    assert_eq!(f(&row["netCashAmount"]), 1050.0);
}

#[test]
fn test_relabel_stored_options_multileg_and_expiry() {
    let d = db();
    d.apply(&[
        opt("opt-ml-1", "2026-08-31T14:16:58Z", "2026-08-31", "OPTIONS_MULTILEG", "FILLED", "LUNR 15JAN27 12.00 CALL", "USD", 0.0, 0.0, -128.0, "other", "OPTIONS_MULTILEG"),
        opt("opt-ml-credit", "2026-08-31T14:16:59Z", "2026-08-31", "OPTIONS_SELL", "SELLTOCLOSE", "BBAI 21JAN28 10.00 CALL", "USD", 0.0, 0.0, 56.0, "trade", "OPTIONS_MULTILEG"),
        opt("opt-exp-1", "2026-08-31T14:17:58Z", "2026-08-31", "OPTIONS_SHORT_EXPIRY", "EXPIRED", "LUNR 15JAN27 12.00 CALL", "USD", 5.0, 0.0, 0.0, "other", "OPTIONS_SHORT_EXPIRY"),
        opt("opt-long-exp", "2026-08-31T14:17:59Z", "2026-08-31", "EXPIR", "BUY", "LUNR 22AUG25 12.00 CALL", "USD", 4.0, 0.0, 0.0, "option_event", "OPTIONS_EXPIRY"),
        opt("opt-asg-1", "2026-08-31T14:18:58Z", "2026-08-31", "OPTIONS_ASSIGN", "ASSIGNED", "LUNR 15JAN27 12.00 CALL", "USD", -2.0, 0.0, 0.0, "other", "OPTIONS_ASSIGN"),
        opt("opt-asg-strike", "2025-03-07T21:00:00Z", "2025-03-07", "ASSIGN", "BUYTOCLOSE", "ASTS 07MAR25 31.00 CALL", "USD", 1.0, 31.0, -3100.0, "option_event", "OPTIONS_ASSIGN"),
    ]);
    d.ensure();
    let m = by_cid(&d);
    let ml = &m["opt-ml-1"];
    assert_eq!((ml["category"].as_str(), ml["activityType"].as_str(), ml["activitySubType"].as_str()), (Some("trade"), Some("OPTIONS_BUY"), Some("BUYTOCLOSE")));
    assert_eq!(f(&ml["quantity"]), 0.0);
    assert_eq!(f(&ml["netCashAmount"]), -128.0);
    let credit = &m["opt-ml-credit"];
    assert_eq!((credit["category"].as_str(), credit["activityType"].as_str(), credit["activitySubType"].as_str()), (Some("trade"), Some("OPTIONS_SELL"), Some("SELLTOOPEN")));
    assert_eq!(f(&credit["netCashAmount"]), 56.0);
    let exp = &m["opt-exp-1"];
    assert_eq!((exp["category"].as_str(), exp["activityType"].as_str(), exp["activitySubType"].as_str()), (Some("option_event"), Some("EXPIR"), Some("BUY")));
    assert_eq!(f(&exp["quantity"]), 5.0);
    let long_exp = &m["opt-long-exp"];
    assert_eq!((long_exp["category"].as_str(), long_exp["activityType"].as_str(), long_exp["activitySubType"].as_str()), (Some("option_event"), Some("EXPIR"), Some("SELL")));
    assert_eq!(f(&long_exp["quantity"]), -4.0);
    let asg = &m["opt-asg-1"];
    assert_eq!((asg["category"].as_str(), asg["activityType"].as_str(), asg["activitySubType"].as_str()), (Some("option_event"), Some("ASSIGN"), Some("BUYTOCLOSE")));
    assert_eq!(f(&asg["quantity"]), 2.0);
    let strike = &m["opt-asg-strike"];
    assert_eq!(f(&strike["unitPrice"]), 0.0);
    assert_eq!(strike["activitySubType"], "BUYTOCLOSE");
}

/// StatusCountsTest. Python counts calls to a patched `_relabel_option_trades`;
/// here the relabel reports whether it ran.
#[test]
fn test_the_option_relabel_runs_once_until_the_rows_change() {
    let d = db();
    d.conn.execute("DELETE FROM meta WHERE key = ?", [relabel::OPTION_RELABEL_META]).unwrap();
    assert!(relabel::relabel_when_rows_changed(&d.conn).unwrap());
    assert!(!relabel::relabel_when_rows_changed(&d.conn).unwrap(), "an unchanged table is relabelled once");
    assert!(!relabel::relabel_when_rows_changed(&d.conn).unwrap());
    d.insert_local(json!({"id": "o1", "transactionDate": "2026-02-02", "symbol": "AAA", "category": "trade", "activitySubType": "BUY", "quantity": 1, "unitPrice": 2.0, "netCashAmount": -2.0, "currency": "CAD"}));
    assert!(relabel::relabel_when_rows_changed(&d.conn).unwrap(), "a new row is relabelled");
    assert!(!relabel::relabel_when_rows_changed(&d.conn).unwrap());
}
