//! Activity mapping, sync bounds and NAV helpers, with no network.

use bagholder_ws::{fetch, mapping, sync};
use rusqlite::Connection;
use serde_json::{json, Value};

fn ws_item(over: Value) -> Value {
    let mut item = json!({
        "occurredAt": "2024-06-15T13:45:22.123Z",
        "canonicalId": "ws-cid-aaa-001",
        "status": "POSTED",
        "type": "DIY_BUY",
        "subType": "BUY",
        "assetSymbol": "AAA",
        "assetQuantity": 10,
        "amount": 100,
        "accountId": "acct-1",
        "currency": "CAD",
    });
    for (k, v) in over.as_object().unwrap() {
        item[k] = v.clone();
    }
    item
}

fn map(item: &Value) -> Value {
    mapping::map_activity(item, None).expect("mapped row")
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {}", v))
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{} != {}", a, b);
}

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
    conn
}

fn apply(conn: &Connection, rows: &[Value]) {
    let n = std::cell::Cell::new(0u64);
    let id = || {
        n.set(n.get() + 1);
        format!("00000000-0000-4000-8000-{:012}", n.get())
    };
    bagholder_store::merge::apply_wealthsimple_mapped(conn, rows, &id).unwrap();
}

#[test]
fn test_options_sell_maps_as_sell_to_open() {
    let row = map(&ws_item(json!({"type": "OPTIONS_SELL", "subType": "LIMIT_ORDER", "assetSymbol": "QNC", "contractType": "CALL", "strikePrice": 3, "expiryDate": "2027-02-19", "assetQuantity": 35, "amount": 1050, "amountSign": "positive"})));
    assert_eq!(row["activitySubType"], "SELLTOOPEN");
    assert_eq!(row["category"], "trade");
    assert_eq!(f(&row["quantity"]), -35.0);
    assert_eq!(f(&row["netCashAmount"]), 1050.0);
    assert_eq!(row["symbol"], "QNC 19FEB27 3.00 CALL");
}

#[test]
fn test_cash_dividend_without_status_is_kept() {
    let mut item = json!({"type": "DIVIDEND", "subType": "CASH_DIVIDEND", "status": null, "amount": "3660.00", "amountSign": "positive", "assetQuantity": "18300.0", "assetSymbol": "RDDY", "currency": "CAD", "occurredAt": "2026-06-05T14:53:21.630000+00:00", "accountId": "non-registered-x", "canonicalId": "div-1"});
    assert!(!mapping::skip_activity(&item));
    let rec = map(&item);
    assert_eq!(rec["category"], "dividend");
    assert_eq!(rec["symbol"], "RDDY");
    approx(f(&rec["netCashAmount"]), 3660.0);
    approx(f(&rec["unitPrice"]), 0.2);
    assert_eq!(rec["transactionDate"], "2026-06-05");
    item["type"] = json!("DIY_BUY");
    assert!(mapping::skip_activity(&item));
}

#[test]
fn test_margin_interest_charge_without_status_is_kept() {
    let item = json!({"type": "INTEREST_CHARGE", "subType": "MARGIN_INTEREST", "status": null, "amount": "412.10", "amountSign": "negative", "currency": "CAD", "occurredAt": "2026-06-01T04:00:00.000000+00:00", "accountId": "non-registered-x", "canonicalId": "int-1"});
    assert!(!mapping::skip_activity(&item));
    let rec = map(&item);
    assert_eq!(rec["activityType"], "INTEREST_CHARGE");
    approx(f(&rec["netCashAmount"]), -412.10);
    assert_eq!(rec["transactionDate"], "2026-06-01");
}

#[test]
fn test_options_buy_maps_as_buy_to_open() {
    let row = map(&ws_item(json!({"type": "OPTIONS_BUY", "subType": "LIMIT_ORDER", "assetSymbol": "QNC", "contractType": "CALL", "strikePrice": 3, "expiryDate": "2027-02-19", "assetQuantity": 5, "amount": 150, "amountSign": "negative"})));
    assert_eq!(row["activitySubType"], "BUYTOOPEN");
    assert_eq!(row["category"], "trade");
    assert_eq!(f(&row["quantity"]), 5.0);
    assert_eq!(f(&row["netCashAmount"]), -150.0);
}

#[test]
fn test_map_activity_options_multileg_debit_is_buy_to_close() {
    let row = map(&ws_item(json!({"type": "OPTIONS_MULTILEG", "subType": "FILLED", "status": "FILLED", "assetSymbol": "LUNR", "contractType": "CALL", "strikePrice": 12, "expiryDate": "2027-01-15", "assetQuantity": null, "amount": 128, "amountSign": "negative", "currency": "USD"})));
    assert_eq!(row["category"], "trade");
    assert_eq!(row["activityType"], "OPTIONS_BUY");
    assert_eq!(row["activitySubType"], "BUYTOCLOSE");
    assert_eq!(f(&row["quantity"]), 0.0);
    assert_eq!(f(&row["unitPrice"]), 0.0);
    assert_eq!(f(&row["netCashAmount"]), -128.0);
    assert_eq!(row["symbol"], "LUNR 15JAN27 12.00 CALL");
}

#[test]
fn test_map_activity_options_multileg_credit_is_sell_to_open() {
    let row = map(&ws_item(json!({"type": "OPTIONS_MULTILEG", "subType": "FILLED", "status": "FILLED", "assetSymbol": "BBAI", "contractType": "CALL", "strikePrice": 10, "expiryDate": "2028-01-21", "assetQuantity": null, "amount": 56, "amountSign": "positive", "currency": "USD"})));
    assert_eq!(row["category"], "trade");
    assert_eq!(row["activityType"], "OPTIONS_SELL");
    assert_eq!(row["activitySubType"], "SELLTOOPEN");
    assert_eq!(f(&row["quantity"]), 0.0);
    assert_eq!(f(&row["unitPrice"]), 0.0);
    assert_eq!(f(&row["netCashAmount"]), 56.0);
    assert_eq!(row["symbol"], "BBAI 21JAN28 10.00 CALL");
}

#[test]
fn test_map_activity_options_short_expiry_covers_short() {
    let row = map(&ws_item(json!({"type": "OPTIONS_SHORT_EXPIRY", "subType": "EXPIRED", "status": "POSTED", "assetSymbol": "LUNR", "contractType": "CALL", "strikePrice": 12, "expiryDate": "2027-01-15", "assetQuantity": 16, "amount": 0, "amountSign": "negative", "currency": "USD"})));
    assert_eq!(row["category"], "option_event");
    assert_eq!(row["activityType"], "EXPIR");
    assert_eq!(row["activitySubType"], "BUY");
    assert_eq!(f(&row["quantity"]), 16.0);
    assert_eq!(f(&row["unitPrice"]), 0.0);
    assert_eq!(f(&row["netCashAmount"]), 0.0);
}

#[test]
fn test_map_activity_options_expiry_sells_long_assign_covers_short() {
    let expiry = map(&ws_item(json!({"type": "OPTIONS_EXPIRY", "subType": "EXPIRED", "assetSymbol": "LUNR", "contractType": "CALL", "strikePrice": 12, "expiryDate": "2025-08-22", "assetQuantity": 4, "amount": 0})));
    assert_eq!(expiry["category"], "option_event");
    assert_eq!(expiry["activityType"], "EXPIR");
    assert_eq!(expiry["activitySubType"], "SELL");
    assert_eq!(f(&expiry["quantity"]), -4.0);
    assert_eq!(f(&expiry["unitPrice"]), 0.0);
    let assign = map(&ws_item(json!({"type": "OPTIONS_ASSIGN", "subType": "ASSIGNED", "assetSymbol": "ASTS", "contractType": "CALL", "strikePrice": 31, "expiryDate": "2025-03-07", "assetQuantity": 1, "amount": 3100, "amountSign": "negative", "currency": "USD"})));
    assert_eq!(assign["category"], "option_event");
    assert_eq!(assign["activityType"], "ASSIGN");
    assert_eq!(assign["activitySubType"], "BUYTOCLOSE");
    assert_eq!(f(&assign["quantity"]), 1.0);
    assert_eq!(f(&assign["unitPrice"]), 0.0);
    assert_eq!(assign["symbol"], "ASTS 07MAR25 31.00 CALL");
}

#[test]
fn test_map_activity_option_unit_price_is_per_share() {
    let cheap = map(&ws_item(json!({"type": "OPTIONS_SELL", "subType": "LIMIT_ORDER", "assetSymbol": "DRAM", "contractType": "CALL", "strikePrice": 1, "expiryDate": "2027-02-19", "assetQuantity": 10, "amount": 112.5, "amountSign": "positive"})));
    approx(f(&cheap["unitPrice"]), 0.1125);
    let pricey = map(&ws_item(json!({"type": "OPTIONS_SELL", "subType": "LIMIT_ORDER", "assetSymbol": "SOXL", "contractType": "CALL", "strikePrice": 20, "expiryDate": "2027-02-19", "assetQuantity": 10, "amount": 13300, "amountSign": "positive"})));
    approx(f(&pricey["unitPrice"]), 13.3);
    // `_is_option` is private in the crate: a share row keeps its plain ticker
    let share_item = ws_item(json!({"amount": 100, "assetQuantity": 10}));
    let share = map(&share_item);
    assert_eq!(share["symbol"], "AAA");
    approx(f(&share["unitPrice"]), 10.0);
}

#[test]
fn test_map_activity_copies_security_id() {
    let row = map(&ws_item(json!({"securityId": "sec-s-abc123"})));
    assert_eq!(row["securityId"], "sec-s-abc123");
}

#[test]
fn test_daily_window_reaches_back_past_rows_filed_under_a_later_day() {
    let conn = db();
    let late = ws_item(json!({"occurredAt": "2026-09-09T01:11:38.000Z", "canonicalId": "ws-cid-card-0909", "id": "ws-card-0909"}));
    apply(&conn, &[map(&late)]);
    let (start, _full) = sync::activity_sync_bounds(&conn).unwrap();
    assert_eq!(start.as_deref(), Some("2026-08-26"));
    let cond = fetch::activity_fetch_condition("acct-1", start.as_deref(), 1_789_000_000);
    assert!(cond["startDate"].as_str().unwrap() < "2026-09-08T04:00:00.000Z", "a dividend filed under the 8th is inside the window");
}

#[test]
fn test_empty_table_full_history_omits_start_date() {
    let conn = db();
    assert_eq!(bagholder_store::activities::activity_count(&conn).unwrap(), 0);
    let (start, full) = sync::activity_sync_bounds(&conn).unwrap();
    assert!(full);
    assert!(start.is_none());
    let cond = fetch::activity_fetch_condition("acct-1", start.as_deref(), 1_789_000_000);
    assert!(cond.get("startDate").is_none());
}

#[test]
fn test_token_refresh_needed_uses_expires_at() {
    let now = 1_700_000_000.0;
    assert!(sync::token_refresh_needed(&json!({"expires_at": now + 60.0, "refresh_token": "r"}), now));
    assert!(!sync::token_refresh_needed(&json!({"expires_at": now + 3600.0, "refresh_token": "r"}), now));
    assert!(sync::token_refresh_needed(&json!({"refresh_token": "r"}), now));
}

#[test]
fn test_only_open_margin_accounts_are_asked_for_buying_power() {
    let accounts = vec![
        json!({"id": "tfsa-1", "unifiedAccountType": "SELF_DIRECTED_TFSA", "status": "open"}),
        json!({"id": "nr-1", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "status": "open"}),
        json!({"id": "nr-2", "unifiedAccountType": "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN", "status": "closed"}),
        json!({"id": "cash-1", "unifiedAccountType": "CASH", "status": "open"}),
        json!({"id": "", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "status": "open"}),
    ];
    assert_eq!(fetch::margin_account_ids(&accounts), vec!["nr-1".to_string()], "a TFSA's buying power is cash, not margin; a closed margin account holds nothing");
}

#[test]
fn test_nav_account_groups_joins_same_nickname() {
    let groups = mapping::nav_account_groups(Some(&json!([
        {"id": "cad-1", "nickname": "TFSA", "currency": "CAD"},
        {"id": "usd-1", "nickname": "TFSA", "currency": "USD"},
        {"id": "rrsp-1", "nickname": "", "unifiedAccountType": "RRSP"},
    ])));
    assert_eq!(groups["TFSA"], json!(["cad-1", "usd-1"]));
    assert_eq!(groups["RRSP"], json!(["rrsp-1"]));
}

#[test]
fn test_nav_points_from_payload_accepts_v2_and_identity() {
    let (ident, _) = fetch::nav_points_from_payload(&json!({"identity": {"financials": {"historicalDaily": {"edges": [{"node": {"date": "2024-02-01", "netLiquidationValue": {"amount": 10, "currency": "CAD"}, "netDeposits": {"amount": 1, "currency": "CAD"}}}], "pageInfo": {}}}}}));
    assert_eq!(f(&ident[0]["equity"]), 10.0);
    assert_eq!(f(&ident[0]["netDeposits"]), 1.0);
    let (acc, _) = fetch::nav_points_from_payload(&json!({"account": {"financials": {"historicalDaily": {"edges": [{"node": {"date": "2024-02-01", "netLiquidationValueV2": {"amount": "20", "currency": "CAD"}, "netDepositsV2": {"amount": "4", "currency": "CAD"}}}], "pageInfo": {}}}}}));
    assert_eq!(f(&acc[0]["equity"]), 20.0);
    assert_eq!(f(&acc[0]["netDeposits"]), 4.0);
}

#[test]
fn test_merge_nav_points_sums_equity_and_deposits() {
    let merged = fetch::merge_nav_points(&[
        vec![json!({"date": "2024-01-01", "equity": 10, "currency": "CAD", "netDeposits": 1})],
        vec![
            json!({"date": "2024-01-01", "equity": 5, "currency": "CAD", "netDeposits": 2}),
            json!({"date": "2024-01-02", "equity": 6, "currency": "CAD"}),
        ],
    ]);
    assert_eq!(merged[0]["date"], "2024-01-01");
    assert_eq!(f(&merged[0]["equity"]), 15.0);
    assert_eq!(f(&merged[0]["netDeposits"]), 3.0);
    assert_eq!(merged[1]["date"], "2024-01-02");
    assert_eq!(f(&merged[1]["equity"]), 6.0);
    assert!(merged[1].get("netDeposits").is_none());
}
