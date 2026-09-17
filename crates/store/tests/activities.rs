//! Wealthsimple rows by canonical id, local rows without one, and the daily
//! pull schedule (tests/test_store.py, StoreTest).

mod common;
use bagholder_store::{activities, admin, merge, orders, tables};
use common::*;
use serde_json::{json, Value};

#[test]
fn test_insert_if_new_by_canonical_id() {
    let d = db();
    let row = ws_row();
    let first = d.apply(&[row.clone()]);
    assert_eq!(first.inserted, 1);
    assert_eq!(d.count(), 1);
    let again = d.apply(&[row]);
    assert_eq!(again.inserted, 0);
    assert_eq!(again.skipped, 1);
    assert_eq!(d.count(), 1);
}

#[test]
fn test_a_row_wealthsimple_revises_replaces_the_stored_copy() {
    let d = db();
    d.apply(&[ws_row()]);
    let stored_id = d.activities()[0]["id"].clone();
    // map_activity(_ws_item(amount=999, assetQuantity=10))
    let mut changed = with(ws_row(), json!({"description": "Buy 10 AAA @ 99.9", "unitPrice": 99.9, "netCashAmount": -999.0}));
    changed["description"] = json!("revised by Wealthsimple");
    let result = d.apply(&[changed.clone()]);
    assert_eq!((result.inserted, result.revised, result.skipped), (0, 1, 0));
    let stored = d.activities();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["canonicalId"], "ws-cid-aaa-001");
    assert_eq!(stored[0]["id"], stored_id, "the stored id, and so the journal key, survives the revision");
    assert_eq!(stored[0]["description"], "revised by Wealthsimple");
    approx(f(&stored[0]["netCashAmount"]), f(&changed["netCashAmount"]));
    let same_again = d.apply(&[changed]);
    assert_eq!((same_again.revised, same_again.skipped), (0, 1), "an unchanged row is not rewritten");
}

#[test]
fn test_placeholder_dividend_becomes_the_paid_dividend() {
    let d = db();
    let placeholder = with(ws_row(), json!({"canonicalId": "div_E002026619494", "occurredAt": "2026-08-31T04:00:00.000Z", "transactionDate": "2026-08-31", "settlementDate": "2026-08-31", "description": "Buy AAA", "direction": "", "quantity": 4000.0, "unitPrice": 0.0, "netCashAmount": -0.0}));
    d.apply(&[placeholder]);
    let paid = with(ws_row(), json!({"canonicalId": "div_E002026619494", "occurredAt": "2026-09-08T14:02:11.000Z", "transactionDate": "2026-09-08", "settlementDate": "2026-09-08", "description": "Buy 4000 AAA @ 0.255", "quantity": 4000.0, "unitPrice": 0.255, "netCashAmount": -1020.0}));
    let result = d.apply(&[paid.clone()]);
    assert_eq!(result.revised, 1);
    let rows = d.activities();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["transactionDate"], "2026-09-08");
    approx(f(&rows[0]["netCashAmount"]), f(&paid["netCashAmount"]));
    approx(f(&rows[0]["netCashAmount"]).abs(), 1020.0);
}

/// The store half: `bagholder.append_manual` (server crate) normalizes the
/// fields to this row and merges it.
#[test]
fn test_manual_has_no_canonical_id() {
    let d = db();
    let row = json!({"id": "64d0ce97-3502-4bcb-9d97-383f23fbb50c", "occurredAt": "2024-07-01", "transactionDate": "2024-07-01", "settlementDate": "2024-07-01", "accountId": "manual", "bookId": "manual", "accountType": "Manual", "activityType": "Trade", "activitySubType": "BUY", "description": "Buy 3 ZZZ @ 12.5", "direction": "DEBIT", "symbol": "ZZZ", "name": "ZZZ", "currency": "CAD", "quantity": 3.0, "unitPrice": 12.5, "commission": 0.0, "netCashAmount": -37.5, "category": "trade", "balance": null, "source": "manual"});
    let id = d.new_id();
    let result = merge::merge_local_rows(&d.conn, &[row], &id).unwrap();
    let act = &result.activities[0];
    assert!(!act["id"].as_str().unwrap().is_empty());
    assert!(!activities::looks_like_homemade_id(act["id"].as_str().unwrap()));
    assert!(matches!(act.get("canonicalId"), None | Some(Value::Null)) || act["canonicalId"] == "" || act["canonicalId"] == false);
    let stored = &d.activities()[0];
    assert!(stored.get("canonicalId").map_or(true, |v| v.is_null()));
    assert_eq!(stored["source"], "manual");
}

#[test]
fn test_csv_has_no_canonical_id() {
    let d = db();
    let saved = d.insert_local(json!({"transactionDate": "2024-07-02", "occurredAt": "2024-07-02", "accountId": "acct-1", "symbol": "ZZZ", "quantity": 4, "unitPrice": 8, "netCashAmount": -32, "activityType": "Trade", "activitySubType": "BUY", "source": "csv", "canonicalId": "do-not-keep-this"}));
    assert!(saved.get("canonicalId").map_or(true, |v| v.is_null()));
    assert_eq!(saved["source"], "csv");
    assert!(!activities::looks_like_homemade_id(saved["id"].as_str().unwrap()));
}

/// The store half; the mapper's own assertions belong to the ws crate.
#[test]
fn test_occurred_at_not_cut_to_date_for_wealthsimple_row() {
    let d = db();
    let row = ws_row();
    assert!(row.get("id").is_none());
    d.apply(&[row]);
    let stored = &d.activities()[0];
    assert_eq!(stored["occurredAt"], "2024-06-15T13:45:22.123Z");
    assert!(stored["occurredAt"].as_str().unwrap().contains('T'));
}

/// Mountain time is UTC-6 on these summer days.
fn mountain(y: i64, m: u32, d: u32, hh: i64, mm: i64) -> i64 {
    bagholder_model::dates::to_days(y, m, d) * 86400 + (hh + 6) * 3600 + mm * 60
}

#[test]
fn test_activity_pull_due_weekdays_at_2pm_mountain() {
    let d = db();
    let due = |t| admin::activity_pull_due(&d.conn, t).unwrap();
    assert!(!due(mountain(2026, 8, 31, 13, 59)));
    assert!(due(mountain(2026, 8, 31, 14, 0)));
    assert!(due(mountain(2026, 8, 31, 15, 0)));
    assert!(!due(mountain(2026, 8, 29, 15, 0)));
    admin::mark_activity_pulled(&d.conn, "2026-08-31T20:05:00Z").unwrap();
    assert!(!due(mountain(2026, 8, 31, 15, 0)));
    assert!(due(mountain(2026, 9, 1, 14, 0)));
}

/// The store half; `activity_sync_bounds` is the ws crate's.
#[test]
fn test_existing_rows_make_daily_sync_incremental() {
    let d = db();
    d.apply(&[ws_row()]);
    d.insert_local(json!({"transactionDate": "2024-07-01", "occurredAt": "2024-07-01", "accountId": "manual", "symbol": "ZZZ", "quantity": 1, "unitPrice": 2, "netCashAmount": -2, "activityType": "Trade", "activitySubType": "BUY", "source": "manual"}));
    assert_eq!(d.count(), 2);
    let all = d.activities();
    let ws = all.iter().find(|a| a["source"] == "wealthsimple").unwrap();
    assert_eq!(ws["canonicalId"], "ws-cid-aaa-001");
    assert!(!activities::looks_like_homemade_id(ws["id"].as_str().unwrap()));
    let manual = all.iter().find(|a| a["source"] == "manual").unwrap();
    assert!(manual.get("canonicalId").map_or(true, |v| v.is_null()));
    assert_eq!(activities::canonical_ids(&d.conn).unwrap(), vec!["ws-cid-aaa-001".to_string()]);
}

#[test]
fn test_link_single_manual_match_stamps_canonical_id() {
    let d = db();
    let manual = json!({"id": "007ede4e-969c-467c-9aa9-31acc30f5b6a", "occurredAt": "2024-06-15", "transactionDate": "2024-06-15", "settlementDate": "2024-06-15", "accountId": "acct-1", "bookId": "acct-1", "accountType": "", "activityType": "Trade", "activitySubType": "BUY", "description": "Buy 10 AAA @ 10", "direction": "DEBIT", "symbol": "AAA", "name": "AAA", "currency": "CAD", "quantity": 10.0, "unitPrice": 10.0, "commission": 0.0, "netCashAmount": -100.0, "category": "trade", "balance": null, "source": "manual"});
    let id = d.new_id();
    merge::merge_local_rows(&d.conn, &[manual], &id).unwrap();
    assert_eq!(d.count(), 1);
    d.apply(&[ws_row()]);
    let rows = d.activities();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["canonicalId"], "ws-cid-aaa-001");
    assert_eq!(rows[0]["source"], "manual");
}

#[test]
fn test_second_sync_stamps_security_id_and_takes_the_revision() {
    let d = db();
    let row = ws_row();
    d.apply(&[row.clone()]);
    let later = with(row, json!({"securityId": "sec-s-later", "quantity": 999, "netCashAmount": 1}));
    let again = d.apply(&[later.clone()]);
    assert_eq!((again.inserted, again.revised, again.skipped), (0, 1, 0));
    let got = d.activities()[0].clone();
    assert_eq!(got["securityId"], "sec-s-later");
    assert_eq!(f(&got["quantity"]), 999.0);
    let unchanged = with(later, json!({"securityId": "sec-s-other"}));
    let again = d.apply(&[unchanged]);
    assert_eq!((again.revised, again.skipped), (0, 1));
    assert_eq!(d.activities()[0]["securityId"], "sec-s-later", "a security id already stored is kept");
    assert_eq!(f(&got["netCashAmount"]), 1.0);
}

/// StatusCountsTest, the store half: `status_counts` is the server crate's;
/// the figures it reads are these.
#[test]
fn test_the_counts_match_the_snapshot_without_reading_the_rows() {
    let d = db();
    for i in 0..4 {
        d.insert_local(json!({"id": format!("m{}", i), "transactionDate": format!("2026-01-0{}", i + 1), "symbol": "AAA", "category": "trade", "activitySubType": "BUY", "quantity": 1, "unitPrice": 2.0, "netCashAmount": -2.0, "currency": "CAD"}));
    }
    tables::replace_accounts(&d.conn, &[json!({"id": "a1", "nickname": "One"}), json!({"id": "a2", "nickname": "Two"})]).unwrap();
    tables::set_meta(&d.conn, "synced_at", "2026-09-12T10:00:00Z").unwrap();
    let snap = d.snapshot();
    assert_eq!(d.count() as usize, snap["activities"].as_array().unwrap().len());
    assert_eq!(tables::accounts(&d.conn).unwrap().len(), snap["accounts"].as_array().unwrap().len());
    assert_eq!(snap["syncedAt"], tables::get_meta(&d.conn, "synced_at", "").unwrap());
}

/// OrderTicketTest: the ticket (server crate) writes this row; the store keeps it.
#[test]
fn test_the_orders_table_survives_clear_synced_data() {
    let d = db();
    orders::insert_order(&d.conn, &json!({"id": "order-1", "accountId": "acct-margin", "securityId": "sec-s-us", "symbol": "QNC", "currency": "USD", "side": "BUY", "type": "LIMIT", "tif": "DAY", "quantity": 25, "limitPrice": 165.4, "status": "dry", "source": "bagholder"}), "2026-09-10T14:00:00Z").unwrap();
    admin::clear_synced_data(&d.conn, false, false).unwrap();
    let rows = orders::list_orders(&d.conn, 200).unwrap();
    assert_eq!(rows.len(), 1, "what was submitted is a record of the user's own actions, never cleared with the synced rows");
    assert_eq!(rows[0]["id"], "order-1");
}
