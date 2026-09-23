//! Activity mapping, sync bounds and NAV helpers, with no network.

use bagholder_ws::wire::ActivityItem;
use bagholder_ws::{fetch, mapping, sync};
use rusqlite::Connection;
use serde_json::{json, Value};

fn ws_item(over: Value) -> ActivityItem {
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
    serde_json::from_value(item).unwrap()
}

fn map(item: &ActivityItem) -> bagholder_store::broker::MappedActivity {
    mapping::map_activity(item, &mapping::Accounts::default()).expect("mapped row")
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{} != {}", a, b);
}

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
    conn
}

fn apply(conn: &Connection, rows: &[bagholder_store::broker::MappedActivity]) {
    let n = std::cell::Cell::new(0u64);
    let id = || {
        n.set(n.get() + 1);
        format!("00000000-0000-4000-8000-{:012}", n.get())
    };
    let rows = bagholder_store::broker::MappedActivity::to_rows(rows);
    bagholder_store::merge::apply_wealthsimple_mapped(conn, &rows, &id).unwrap();
}

#[test]
fn test_options_sell_maps_as_sell_to_open() {
    let row = map(&ws_item(json!({"type": "OPTIONS_SELL", "subType": "LIMIT_ORDER", "assetSymbol": "QNC", "contractType": "CALL", "strikePrice": 3, "expiryDate": "2027-02-19", "assetQuantity": 35, "amount": 1050, "amountSign": "positive"})));
    assert_eq!(row.activity_sub_type, "SELLTOOPEN");
    assert_eq!(row.category, "trade");
    assert_eq!(row.quantity, -35.0);
    assert_eq!(row.net_cash_amount, 1050.0);
    assert_eq!(row.symbol, "QNC 19FEB27 3.00 CALL");
}

#[test]
fn test_cash_dividend_without_status_is_kept() {
    let item = ws_item(json!({"type": "DIVIDEND", "subType": "CASH_DIVIDEND", "status": null, "amount": "3660.00", "amountSign": "positive", "assetQuantity": "18300.0", "assetSymbol": "RDDY", "currency": "CAD", "occurredAt": "2026-06-05T14:53:21.630000+00:00", "accountId": "non-registered-x", "canonicalId": "div-1"}));
    assert!(!mapping::skip_activity(&item));
    let rec = map(&item);
    assert_eq!(rec.category, "dividend");
    assert_eq!(rec.symbol, "RDDY");
    approx(rec.net_cash_amount, 3660.0);
    approx(rec.unit_price, 0.2);
    assert_eq!(rec.transaction_date, "2026-06-05");
    let mut item2 = item;
    item2.kind = "DIY_BUY".into();
    assert!(mapping::skip_activity(&item2));
}

#[test]
fn test_margin_interest_charge_without_status_is_kept() {
    let item = ws_item(json!({"type": "INTEREST_CHARGE", "subType": "MARGIN_INTEREST", "status": null, "amount": "412.10", "amountSign": "negative", "currency": "CAD", "occurredAt": "2026-06-01T04:00:00.000000+00:00", "accountId": "non-registered-x", "canonicalId": "int-1"}));
    assert!(!mapping::skip_activity(&item));
    let rec = map(&item);
    assert_eq!(rec.activity_type, "INTEREST_CHARGE");
    approx(rec.net_cash_amount, -412.10);
    assert_eq!(rec.transaction_date, "2026-06-01");
}

#[test]
fn test_options_buy_maps_as_buy_to_open() {
    let row = map(&ws_item(json!({"type": "OPTIONS_BUY", "subType": "LIMIT_ORDER", "assetSymbol": "QNC", "contractType": "CALL", "strikePrice": 3, "expiryDate": "2027-02-19", "assetQuantity": 5, "amount": 150, "amountSign": "negative"})));
    assert_eq!(row.activity_sub_type, "BUYTOOPEN");
    assert_eq!(row.category, "trade");
    assert_eq!(row.quantity, 5.0);
    assert_eq!(row.net_cash_amount, -150.0);
}

#[test]
fn test_map_activity_options_multileg_debit_is_buy_to_close() {
    let row = map(&ws_item(json!({"type": "OPTIONS_MULTILEG", "subType": "FILLED", "status": "FILLED", "assetSymbol": "LUNR", "contractType": "CALL", "strikePrice": 12, "expiryDate": "2027-01-15", "assetQuantity": null, "amount": 128, "amountSign": "negative", "currency": "USD"})));
    assert_eq!(row.category, "trade");
    assert_eq!(row.activity_type, "OPTIONS_BUY");
    assert_eq!(row.activity_sub_type, "BUYTOCLOSE");
    assert_eq!(row.quantity, 0.0);
    assert_eq!(row.unit_price, 0.0);
    assert_eq!(row.net_cash_amount, -128.0);
    assert_eq!(row.symbol, "LUNR 15JAN27 12.00 CALL");
}

#[test]
fn test_map_activity_options_multileg_credit_is_sell_to_open() {
    let row = map(&ws_item(json!({"type": "OPTIONS_MULTILEG", "subType": "FILLED", "status": "FILLED", "assetSymbol": "BBAI", "contractType": "CALL", "strikePrice": 10, "expiryDate": "2028-01-21", "assetQuantity": null, "amount": 56, "amountSign": "positive", "currency": "USD"})));
    assert_eq!(row.category, "trade");
    assert_eq!(row.activity_type, "OPTIONS_SELL");
    assert_eq!(row.activity_sub_type, "SELLTOOPEN");
    assert_eq!(row.quantity, 0.0);
    assert_eq!(row.unit_price, 0.0);
    assert_eq!(row.net_cash_amount, 56.0);
    assert_eq!(row.symbol, "BBAI 21JAN28 10.00 CALL");
}

#[test]
fn test_map_activity_options_short_expiry_covers_short() {
    let row = map(&ws_item(json!({"type": "OPTIONS_SHORT_EXPIRY", "subType": "EXPIRED", "status": "POSTED", "assetSymbol": "LUNR", "contractType": "CALL", "strikePrice": 12, "expiryDate": "2027-01-15", "assetQuantity": 16, "amount": 0, "amountSign": "negative", "currency": "USD"})));
    assert_eq!(row.category, "option_event");
    assert_eq!(row.activity_type, "EXPIR");
    assert_eq!(row.activity_sub_type, "BUY");
    assert_eq!(row.quantity, 16.0);
    assert_eq!(row.unit_price, 0.0);
    assert_eq!(row.net_cash_amount, 0.0);
}

#[test]
fn test_map_activity_options_expiry_sells_long_assign_covers_short() {
    let expiry = map(&ws_item(json!({"type": "OPTIONS_EXPIRY", "subType": "EXPIRED", "assetSymbol": "LUNR", "contractType": "CALL", "strikePrice": 12, "expiryDate": "2025-08-22", "assetQuantity": 4, "amount": 0})));
    assert_eq!(expiry.category, "option_event");
    assert_eq!(expiry.activity_type, "EXPIR");
    assert_eq!(expiry.activity_sub_type, "SELL");
    assert_eq!(expiry.quantity, -4.0);
    assert_eq!(expiry.unit_price, 0.0);
    let assign = map(&ws_item(json!({"type": "OPTIONS_ASSIGN", "subType": "ASSIGNED", "assetSymbol": "ASTS", "contractType": "CALL", "strikePrice": 31, "expiryDate": "2025-03-07", "assetQuantity": 1, "amount": 3100, "amountSign": "negative", "currency": "USD"})));
    assert_eq!(assign.category, "option_event");
    assert_eq!(assign.activity_type, "ASSIGN");
    assert_eq!(assign.activity_sub_type, "BUYTOCLOSE");
    assert_eq!(assign.quantity, 1.0);
    assert_eq!(assign.unit_price, 0.0);
    assert_eq!(assign.symbol, "ASTS 07MAR25 31.00 CALL");
}

#[test]
fn test_map_activity_option_unit_price_is_per_share() {
    let cheap = map(&ws_item(json!({"type": "OPTIONS_SELL", "subType": "LIMIT_ORDER", "assetSymbol": "DRAM", "contractType": "CALL", "strikePrice": 1, "expiryDate": "2027-02-19", "assetQuantity": 10, "amount": 112.5, "amountSign": "positive"})));
    approx(cheap.unit_price, 0.1125);
    let pricey = map(&ws_item(json!({"type": "OPTIONS_SELL", "subType": "LIMIT_ORDER", "assetSymbol": "SOXL", "contractType": "CALL", "strikePrice": 20, "expiryDate": "2027-02-19", "assetQuantity": 10, "amount": 13300, "amountSign": "positive"})));
    approx(pricey.unit_price, 13.3);
    // `_is_option` is private in the crate: a share row keeps its plain ticker
    let share_item = ws_item(json!({"amount": 100, "assetQuantity": 10}));
    let share = map(&share_item);
    assert_eq!(share.symbol, "AAA");
    approx(share.unit_price, 10.0);
}

#[test]
fn test_map_activity_copies_security_id() {
    let row = map(&ws_item(json!({"securityId": "sec-s-abc123"})));
    assert_eq!(row.security_id.as_deref(), Some("sec-s-abc123"));
}

#[test]
fn test_daily_window_reaches_back_past_rows_filed_under_a_later_day() {
    let conn = db();
    let late = ws_item(json!({"occurredAt": "2026-09-09T01:11:38.000Z", "canonicalId": "ws-cid-card-0909", "id": "ws-card-0909"}));
    apply(&conn, &[map(&late)]);
    let (start, _full) = sync::activity_sync_bounds(&conn).unwrap();
    assert_eq!(start.as_deref(), Some("2026-08-26"));
    let cond = fetch::activity_fetch_condition("acct-1", start.as_deref(), 1_789_000_000);
    assert!(cond.start_date.as_deref().unwrap() < "2026-09-08T04:00:00.000Z", "a dividend filed under the 8th is inside the window");
}

#[test]
fn test_empty_table_full_history_omits_start_date() {
    let conn = db();
    assert_eq!(bagholder_store::activities::activity_count(&conn).unwrap(), 0);
    let (start, full) = sync::activity_sync_bounds(&conn).unwrap();
    assert!(full);
    assert!(start.is_none());
    let cond = fetch::activity_fetch_condition("acct-1", start.as_deref(), 1_789_000_000);
    assert!(cond.start_date.is_none());
}

#[test]
fn test_token_refresh_needed_uses_expires_at() {
    let now = 1_700_000_000.0;
    let sess = |expires_at: Option<f64>| {
        let mut s = bagholder_ws::session::Session { refresh_token: "r".into(), ..Default::default() };
        s.expires_at = expires_at.map(bagholder_ws::session::Expiry::Unix);
        s
    };
    assert!(sync::token_refresh_needed(&sess(Some(now + 60.0)), now));
    assert!(!sync::token_refresh_needed(&sess(Some(now + 3600.0)), now));
    assert!(sync::token_refresh_needed(&sess(None), now));
}

#[test]
fn test_only_open_margin_accounts_are_asked_for_buying_power() {
    let node = |id: &str, typ: &str, status: &str| -> bagholder_ws::wire::AccountNode {
        serde_json::from_value(json!({"id": id, "unifiedAccountType": typ, "status": status})).unwrap()
    };
    let accounts = vec![
        node("tfsa-1", "SELF_DIRECTED_TFSA", "open"),
        node("nr-1", "SELF_DIRECTED_NON_REGISTERED_MARGIN", "open"),
        node("nr-2", "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN", "closed"),
        node("cash-1", "CASH", "open"),
        node("", "SELF_DIRECTED_NON_REGISTERED_MARGIN", "open"),
    ];
    assert_eq!(fetch::margin_account_ids(&accounts), vec!["nr-1".to_string()], "a TFSA's buying power is cash, not margin; a closed margin account holds nothing");
}

#[test]
fn test_nav_account_groups_joins_same_nickname() {
    let nodes: Vec<bagholder_ws::wire::AccountNode> = serde_json::from_value(json!([
        {"id": "cad-1", "nickname": "TFSA", "currency": "CAD"},
        {"id": "usd-1", "nickname": "TFSA", "currency": "USD"},
        {"id": "rrsp-1", "nickname": "", "unifiedAccountType": "RRSP"},
    ])).unwrap();
    let groups = mapping::nav_account_groups(&nodes);
    assert_eq!(groups["TFSA"], vec!["cad-1".to_string(), "usd-1".to_string()]);
    assert_eq!(groups["RRSP"], vec!["rrsp-1".to_string()]);
}

#[test]
fn test_nav_points_from_payload_accepts_v2_and_identity() {
    let ident_answer: bagholder_ws::wire::NavAnswer = serde_json::from_value(json!({"identity": {"financials": {"historicalDaily": {"edges": [{"node": {"date": "2024-02-01", "netLiquidationValue": {"amount": 10, "currency": "CAD"}, "netDeposits": {"amount": 1, "currency": "CAD"}}}], "pageInfo": {}}}}})).unwrap();
    let (ident, _) = fetch::nav_points_from_payload(&ident_answer);
    assert_eq!(ident[0].equity, Some(10.0));
    assert_eq!(ident[0].net_deposits, Some(1.0));
    let acc_answer: bagholder_ws::wire::NavAnswer = serde_json::from_value(json!({"account": {"financials": {"historicalDaily": {"edges": [{"node": {"date": "2024-02-01", "netLiquidationValueV2": {"amount": "20", "currency": "CAD"}, "netDepositsV2": {"amount": "4", "currency": "CAD"}}}], "pageInfo": {}}}}})).unwrap();
    let (acc, _) = fetch::nav_points_from_payload(&acc_answer);
    assert_eq!(acc[0].equity, Some(20.0));
    assert_eq!(acc[0].net_deposits, Some(4.0));
}

#[test]
fn test_merge_nav_points_sums_equity_and_deposits() {
    let point = |date: &str, equity: f64, currency: &str, net_deposits: Option<f64>| bagholder_store::broker::NavPoint {
        account_id: String::new(),
        date: date.into(),
        equity: Some(equity),
        currency: currency.into(),
        net_deposits,
    };
    let merged = fetch::merge_nav_points(&[
        vec![point("2024-01-01", 10.0, "CAD", Some(1.0))],
        vec![point("2024-01-01", 5.0, "CAD", Some(2.0)), point("2024-01-02", 6.0, "CAD", None)],
    ]);
    assert_eq!(merged[0].date, "2024-01-01");
    assert_eq!(merged[0].equity, Some(15.0));
    assert_eq!(merged[0].net_deposits, Some(3.0));
    assert_eq!(merged[1].date, "2024-01-02");
    assert_eq!(merged[1].equity, Some(6.0));
    assert!(merged[1].net_deposits.is_none());
}
