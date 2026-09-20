//! An order and a bracket, written and read as what they are.

mod common;

use bagholder_store::orders::typed;
use bagholder_store::orders::{self, Bracket, BracketPatch, BracketStatus, Order, OrderPatch, OrderStatus, OrderType, Role, Side, SlKind, SlMode, Source, StopLoss, TakeProfit, TrailUnit};
use common::*;
use serde_json::json;

const NOW: &str = "2026-09-10T14:00:00Z";

fn ticket() -> Order {
    Order {
        id: "order-1".into(),
        account_id: "acct-margin".into(),
        account: "Margin".into(),
        security_id: "sec-s-us".into(),
        symbol: "QNC".into(),
        currency: "USD".into(),
        side: Side::Buy,
        kind: OrderType::Limit,
        quantity: Some(25.0),
        limit_price: Some(165.4),
        tif: "DAY".into(),
        stop_loss: Some(StopLoss { kind: SlKind::Trail, price: None, trail: Some(4.5), trail_unit: TrailUnit::Pct }),
        take_profit: Some(TakeProfit { price: Some(190.0) }),
        request: json!({"quantity": 25, "executionType": "LIMIT"}),
        ..Order::default()
    }
}

#[test]
fn test_an_order_reads_back_as_it_was_written_with_what_the_store_gives_it() {
    let d = db();
    typed::insert_order(&d.conn, &ticket(), NOW).unwrap();
    let got = typed::get_order(&d.conn, "order-1").unwrap().unwrap();
    let want = Order { created_at: NOW.into(), updated_at: NOW.into(), source: Source::Bagholder, role: Role::Entry, request: json!({"executionType": "LIMIT", "quantity": 25}), ..ticket() };
    assert_eq!(got, want);
    assert!(typed::get_order(&d.conn, "order-none").unwrap().is_none());
}

#[test]
fn test_a_patch_touches_the_fields_it_carries_and_no_others() {
    let d = db();
    typed::insert_order(&d.conn, &ticket(), NOW).unwrap();
    let later = "2026-09-10T14:05:00Z";
    let patch = OrderPatch { status: Some(OrderStatus::Sent), ws_order_id: Some("ws-9".into()), limit_price: Some(None), ..OrderPatch::default() };
    typed::update_order(&d.conn, "order-1", &patch, later).unwrap();
    let got = typed::get_order(&d.conn, "order-1").unwrap().unwrap();
    let mut want = Order { created_at: NOW.into(), updated_at: later.into(), request: got.request.clone(), ..ticket() };
    patch.apply(&mut want);
    assert_eq!(got, want);
    assert_eq!(got.limit_price, None);
    assert_eq!(got.quantity, Some(25.0));
    // nothing to set: not even the stamp moves
    typed::update_order(&d.conn, "order-1", &OrderPatch::default(), "2026-09-11T00:00:00Z").unwrap();
    assert_eq!(typed::get_order(&d.conn, "order-1").unwrap().unwrap().updated_at, later);
}

#[test]
fn test_the_json_view_of_an_order_is_the_typed_one() {
    let d = db();
    orders::insert_order(&d.conn, &json!({"id": "order-2", "accountId": "a", "symbol": "ENB", "side": "SELL", "type": "MARKET", "quantity": "10", "status": "dry", "source": "manual"}), NOW).unwrap();
    let typed_row = typed::get_order(&d.conn, "order-2").unwrap().unwrap();
    assert_eq!((typed_row.side, typed_row.kind, typed_row.quantity, typed_row.status, typed_row.source), (Side::Sell, OrderType::Market, Some(10.0), OrderStatus::Dry, Source::Manual));
    let as_json = orders::get_order(&d.conn, "order-2").unwrap().unwrap();
    assert_eq!(as_json, serde_json::to_value(&typed_row).unwrap());
    // the wire's words: absent text is "", an absent number null, the keys in this order
    assert_eq!(as_json["limitPrice"], json!(null));
    assert_eq!(as_json["wsOrderId"], json!(""));
    assert_eq!(as_json["stopLoss"], json!(null));
    let keys: Vec<&str> = as_json.as_object().unwrap().keys().map(|k| k.as_str()).take(10).collect();
    assert_eq!(keys, ["id", "createdAt", "accountId", "account", "securityId", "symbol", "currency", "side", "type", "quantity"]);
}

#[test]
fn test_every_word_the_order_code_writes_is_kept_as_written() {
    // a word missing from its enum would be stored as nothing: each is written through
    // the JSON door the order code still uses, and read back
    let d = db();
    for (n, status) in ["dry", "sending", "sent", "pending", "cancelling", "filled", "cancelled", "expired", "rejected", "failed"].iter().enumerate() {
        let id = format!("order-w{}", n);
        orders::insert_order(&d.conn, &json!({"id": id, "status": status, "role": "target", "source": "wealthsimple", "side": "SELL", "type": "STOP_LIMIT"}), NOW).unwrap();
        let row = orders::get_order(&d.conn, &id).unwrap().unwrap();
        assert_eq!((row["status"].as_str(), row["role"].as_str(), row["source"].as_str(), row["side"].as_str(), row["type"].as_str()),
            (Some(*status), Some("target"), Some("wealthsimple"), Some("SELL"), Some("STOP_LIMIT")));
    }
    for (n, status) in ["waiting", "armed", "firing", "target_placed", "stopping", "closing", "done", "cancelled"].iter().enumerate() {
        let id = format!("br-w{}", n);
        orders::insert_bracket(&d.conn, &json!({"id": id, "orderId": "o", "status": status, "slKind": "trail", "slTrailUnit": "amt", "slMode": "watched"}), NOW).unwrap();
        let row = orders::get_bracket(&d.conn, &id).unwrap().unwrap();
        assert_eq!((row["status"].as_str(), row["slKind"].as_str(), row["slTrailUnit"].as_str(), row["slMode"].as_str()), (Some(*status), Some("trail"), Some("amt"), Some("watched")));
    }
}

#[test]
fn test_a_word_this_build_does_not_know_reads_as_not_set() {
    let d = db();
    typed::insert_order(&d.conn, &ticket(), NOW).unwrap();
    d.conn.execute("UPDATE orders SET status = 'teleported', role = '' WHERE id = 'order-1'", []).unwrap();
    let got = typed::get_order(&d.conn, "order-1").unwrap().unwrap();
    assert_eq!(got.status, OrderStatus::Unset);
    assert!(!got.status.is_live());
    assert_eq!(got.role, Role::Entry);
}

#[test]
fn test_a_bracket_reads_back_is_patched_and_is_found_by_status_and_by_its_entry() {
    let d = db();
    let b = Bracket {
        id: "br-1".into(), order_id: "order-1".into(), account_id: "acct-margin".into(), security_id: "sec-s-us".into(),
        symbol: "QNC".into(), currency: "USD".into(), quantity: Some(25.0), tif: "GTC".into(),
        sl_kind: SlKind::Stop, sl_price: Some(150.0), tp_price: Some(190.0), ..Bracket::default()
    };
    typed::insert_bracket(&d.conn, &b, NOW).unwrap();
    let got = typed::get_bracket(&d.conn, "br-1").unwrap().unwrap();
    assert_eq!(got, Bracket { created_at: NOW.into(), updated_at: NOW.into(), status: BracketStatus::Waiting, sl_trail_unit: TrailUnit::Pct, ..b.clone() });

    let patch = BracketPatch { status: Some(BracketStatus::Armed), sl_mode: Some(SlMode::Native), sl_native: Some(true), sl_order_id: Some("order-sl".into()), attempts: Some(2), high_water: Some(Some(171.25)), ..BracketPatch::default() };
    typed::update_bracket(&d.conn, "br-1", &patch, NOW).unwrap();
    let armed = typed::get_bracket(&d.conn, "br-1").unwrap().unwrap();
    let mut want = got.clone();
    patch.apply(&mut want);
    assert_eq!(armed, want);
    assert!(armed.status.is_live());

    assert_eq!(typed::list_brackets(&d.conn, &[BracketStatus::Armed]).unwrap().len(), 1);
    assert_eq!(typed::list_brackets(&d.conn, &[BracketStatus::Waiting]).unwrap().len(), 0);
    assert_eq!(typed::list_brackets(&d.conn, &[]).unwrap().len(), 1);
    assert_eq!(typed::bracket_for_order(&d.conn, "order-1").unwrap().unwrap().id, "br-1");
    assert_eq!(orders::get_bracket(&d.conn, "br-1").unwrap().unwrap(), serde_json::to_value(&armed).unwrap());
}

#[test]
fn test_the_header_counts_the_orders_still_with_the_broker() {
    let d = db();
    for (id, status) in [("o1", OrderStatus::Sent), ("o2", OrderStatus::Pending), ("o3", OrderStatus::Filled), ("o4", OrderStatus::Dry)] {
        typed::insert_order(&d.conn, &Order { id: id.into(), status, ..ticket() }, NOW).unwrap();
    }
    let live = [OrderStatus::Sent, OrderStatus::Pending, OrderStatus::Cancelling];
    assert_eq!(typed::open_orders_count(&d.conn, &live).unwrap(), 2);
    assert_eq!(typed::open_orders_count(&d.conn, &[]).unwrap(), 0);
}
