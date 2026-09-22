//! The bracket engine:
//! the stop loss and take profit after the fill, against a fake Wealthsimple.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{json, Value};

use bagholder_store::orders as so;
use bagholder_store::tables as tb;
use bagholder_ws::session::CallError;

use crate::orders::{self as od, bracket_seam, seam};
use crate::tests_common::app;

static SENT: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());
static REJECTIONS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn lk<T>(m: &'static Mutex<T>) -> MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn db() -> bagholder_store::pool::Pooled<'static> {
    crate::tests_common::app_ref().open().unwrap()
}

fn now() -> String {
    crate::app::now_iso()
}

fn sv(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(x) => x.to_string(),
    }
}

fn fv(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(f64::NAN)
}

fn tv(v: &Value, k: &str) -> bool {
    crate::app::truthy(v.get(k))
}

fn get_bracket(id: &str) -> Value {
    so::get_bracket(&db(), id).unwrap().expect("bracket")
}

fn get_order(id: &str) -> Value {
    so::get_order(&db(), id).unwrap().unwrap_or(json!({}))
}

fn update_order(id: &str, patch: Value) {
    so::update_order(&db(), id, &patch, &now()).unwrap()
}

fn update_bracket(id: &str, patch: Value) {
    so::update_bracket(&db(), id, &patch, &now()).unwrap()
}

fn list_orders() -> Vec<Value> {
    so::list_orders(&db(), 200).unwrap()
}

fn set_meta(k: &str, v: &str) {
    tb::set_meta(&db(), k, v).unwrap()
}

fn replace_balances(rows: Value) {
    tb::replace_balances(&db(), rows.as_array().unwrap()).unwrap()
}

fn fake(op: &str, vars: &Value) -> Result<Value, CallError> {
    let n = {
        let mut s = lk(&SENT);
        s.push((op.to_string(), vars.clone()));
        s.len()
    };
    Ok(match op {
        "SoOrdersOrderCreate" => {
            let mut r = lk(&REJECTIONS);
            if !r.is_empty() {
                let m = r.remove(0);
                json!({"soOrdersCreateOrder": {"errors": [{"code": "x", "message": m}], "order": null}})
            } else {
                json!({"soOrdersCreateOrder": {"errors": [], "order": {"orderId": format!("ws-{}", n), "createdAt": "2026-09-10T13:30:00Z"}}})
            }
        }
        "SoOrdersOrderCancel" => json!({"orderServiceCancelOrder": {"externalId": vars["cancelOrderRequest"]["externalId"], "errors": []}}),
        "FetchSecurityMarketData" => json!({"security": {"id": vars["id"], "allowedOrderSubtypes": ["MARKET", "LIMIT", "STOP", "STOP_LIMIT"], "marginRates": {"clientMarginRate": 0.3}}}),
        "FetchSoOrdersExtendedOrder" => {
            let row = so::get_order(&db(), vars["externalId"].as_str().unwrap_or("")).unwrap().unwrap_or(json!({}));
            let ws = match sv(&row, "status").as_str() {
                "cancelling" => "CANCEL_PENDING",
                "filled" => "FILLED",
                "cancelled" => "CANCELLED",
                "expired" => "EXPIRED",
                "rejected" => "REJECTED",
                _ => "SUBMITTED",
            };
            let exp = if tv(&row, "expiresAt") { row["expiresAt"].clone() } else { Value::Null };
            json!({"soOrdersExtendedOrder": {"status": ws, "filledQuantity": row.get("filledQty").cloned().unwrap_or(Value::Null),
                "averageFilledPrice": row.get("avgFill").cloned().unwrap_or(Value::Null), "submittedQuantity": row.get("quantity").cloned().unwrap_or(Value::Null),
                "timeInForce": row.get("tif").cloned().unwrap_or(Value::Null), "expiredAtUtc": exp}})
        }
        other => panic!("{}", other),
    })
}

fn live(on: bool) {
    *lk(&seam::LIVE) = Some(on);
}

/// A fresh book and a live fake Wealthsimple.
fn setup() -> MutexGuard<'static, ()> {
    let g = crate::tests_common::guard();
    seam::reset();
    bracket_seam::reset(crate::tests_common::app_ref());
    lk(&SENT).clear();
    lk(&REJECTIONS).clear();
    let c = db();
    bagholder_store::schema::init_schema(&c).unwrap();
    for t in ["orders", "brackets", "activities", "balances", "margin", "securities", "accounts"] {
        c.execute(&format!("DELETE FROM {}", t), []).unwrap();
    }
    tb::replace_accounts(&c, json!([
        {"id": "acct-margin", "nickname": "Trading", "unifiedAccountType": "SELF_DIRECTED_NON_REGISTERED_MARGIN", "currency": "CAD", "status": "open", "type": "non_registered"},
        {"id": "acct-tfsa", "nickname": "TFSA", "unifiedAccountType": "SELF_DIRECTED_TFSA", "currency": "CAD", "status": "open", "type": "tfsa", "marginAccountId": "acct-margin"},
        {"id": "acct-crypto", "nickname": "Crypto", "unifiedAccountType": "SELF_DIRECTED_CRYPTO", "currency": "CAD", "status": "open", "type": "crypto"},
        {"id": "acct-old", "nickname": "Old", "unifiedAccountType": "SELF_DIRECTED_RRSP", "currency": "CAD", "status": "closed", "type": "rrsp"},
        {"id": "acct-managed", "nickname": "Managed", "unifiedAccountType": "MANAGED_TFSA", "currency": "CAD", "status": "open", "type": "tfsa"}
    ]).as_array().unwrap()).unwrap();
    bagholder_store::admin::upsert_securities(&c, json!([
        {"id": "sec-o-1", "symbol": "QNC", "name": "", "primaryExchange": "", "primaryMic": "", "currency": "USD", "underlyingId": "sec-s-us"},
        {"id": "sec-s-us", "symbol": "QNC", "name": "Quantum Emotion Corp", "primaryExchange": "NYSE", "primaryMic": "XNYS", "currency": "USD", "underlyingId": null},
        {"id": "sec-s-ca", "symbol": "QNC.TO", "name": "Quantum Emotion Corp", "primaryExchange": "TSX-V", "primaryMic": "XTSX", "currency": "CAD", "underlyingId": null}
    ]).as_array().unwrap(), &now()).unwrap();
    tb::replace_margin(&c, json!([{"accountId": "acct-margin", "buyingPower": 12680.45, "currency": "CAD"}]).as_array().unwrap(), &now()).unwrap();
    tb::replace_balances(&c, json!([{"accountId": "acct-margin", "securityId": "sec-s-us", "quantity": 25}]).as_array().unwrap()).unwrap();
    tb::set_meta(&c, "balances_read_at", "").unwrap();
    *lk(&seam::GQL) = Some(Arc::new(fake));
    *lk(&seam::SESSION) = Some(Some(json!({"access_token": "t"})));
    live(true);
    g
}

fn ticket(over: Value) -> Value {
    let mut body = json!({"symbol": "QNC", "securityId": "sec-s-us", "accountId": "acct-margin", "side": "BUY", "type": "LIMIT", "tif": "DAY",
        "quantity": 25, "limitPrice": 165.4, "stopPrice": null, "currency": "USD",
        "stopLoss": {"kind": "stop", "price": 157.13}, "takeProfit": {"price": 181.94}});
    for (k, v) in over.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

fn entry(over: Value) -> (String, Value) {
    let r = od::place_order(&app(), &ticket(over));
    assert!(tv(&r, "ok"), "{}", r);
    (sv(&r, "id"), get_bracket(&sv(&r, "bracketId")))
}

fn q(last: f64, bid: Option<f64>, status: &str) -> Value {
    json!({"last": last, "bid": bid.unwrap_or(last), "ask": last + 0.02, "marketStatus": status})
}

fn tick(quote: Option<Value>) -> Value {
    let mut m = HashMap::new();
    if let Some(v) = quote {
        m.insert("sec-s-us".to_string(), v);
    }
    od::bracket_tick(&app(), Some(m))
}

fn t0() {
    tick(None);
}

fn tq(last: f64, bid: f64) {
    tick(Some(q(last, Some(bid), "OPEN")));
}

fn ops() -> Vec<String> {
    lk(&SENT).iter().map(|(o, _)| o.clone()).collect()
}

fn clear() {
    lk(&SENT).clear();
}

fn sent_empty() -> bool {
    lk(&SENT).is_empty()
}

fn creates() -> Vec<Value> {
    lk(&SENT).iter().filter(|(o, _)| o == "SoOrdersOrderCreate").map(|(_, v)| v["input"].clone()).collect()
}

fn cancels() -> Vec<String> {
    lk(&SENT).iter().filter(|(o, _)| o == "SoOrdersOrderCancel").map(|(_, v)| sv(&v["cancelOrderRequest"], "externalId")).collect()
}

fn filled(oid: &str) {
    update_order(oid, json!({"status": "filled", "filledQty": 25}));
}

#[test]
fn test_a_ticket_with_brackets_makes_a_waiting_bracket() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    assert_eq!((sv(&b, "orderId"), sv(&b, "status"), sv(&b, "slKind"), fv(&b, "slPrice"), fv(&b, "tpPrice"), fv(&b, "quantity"), sv(&b, "tif")),
        (oid, "waiting".into(), "stop".into(), 157.13, 181.94, 25.0, "UNTIL_CANCEL".into()));
    assert!(so::bracket_for_order(&db(), "nope").unwrap().is_none());
    t0();
    assert_eq!(sv(&get_bracket(&sv(&b, "id")), "status"), "waiting", "an unfilled entry leaves the bracket waiting");
}

#[test]
fn test_the_fill_arms_the_bracket_and_places_the_stop_as_wealthsimples_own_order() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    update_order(&oid, json!({"status": "filled", "filledQty": 25, "avgFill": 165.38}));
    clear();
    t0();
    let b = get_bracket(&sv(&b, "id"));
    assert_eq!((sv(&b, "status"), sv(&b, "slMode"), b["slNative"].clone()), ("armed".into(), "native".into(), json!(true)));
    assert!(sv(&b, "slOrderId").starts_with("order-"));
    let c = creates();
    assert_eq!(c.len(), 1);
    let mut inp = c[0].clone();
    inp.as_object_mut().unwrap().remove("externalId");
    let want = json!({"canonicalAccountId": "acct-margin", "executionType": "STOP", "orderType": "SELL_QUANTITY", "quantity": 25.0, "securityId": "sec-s-us", "timeInForce": "UNTIL_CANCEL", "stopPrice": 157.13});
    assert_eq!(inp.as_object().unwrap().len(), want.as_object().unwrap().len(), "{}", inp);
    for (k, v) in want.as_object().unwrap() {
        if v.is_number() {
            assert_eq!(inp[k].as_f64(), v.as_f64(), "{}", k);
        } else {
            assert_eq!(&inp[k], v, "{}", k);
        }
    }
    let stop = get_order(&sv(&b, "slOrderId"));
    assert_eq!((sv(&stop, "role"), sv(&stop, "parentId"), sv(&stop, "side"), sv(&stop, "type"), sv(&stop, "status")),
        ("stop".into(), oid, "SELL".into(), "STOP".into(), "sent".into()));
    clear();
    tick(Some(q(170.0, None, "OPEN")));
    assert!(ops().is_empty());
}

#[test]
fn test_a_partial_fill_at_the_end_arms_for_what_filled() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    update_order(&oid, json!({"status": "cancelled", "filledQty": 10}));
    t0();
    let b = get_bracket(&sv(&b, "id"));
    assert_eq!((sv(&b, "status"), fv(&b, "quantity")), ("armed".into(), 10.0));
    assert_eq!(fv(&get_order(&sv(&b, "slOrderId")), "quantity"), 10.0);
}

#[test]
fn test_an_entry_that_never_filled_ends_the_bracket() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    update_order(&oid, json!({"status": "cancelled"}));
    t0();
    let b = get_bracket(&sv(&b, "id"));
    assert_eq!((sv(&b, "status"), sv(&b, "outcome")), ("cancelled".into(), "entry cancelled".into()));
}

#[test]
fn test_the_target_cancels_the_stop_then_places_the_limit_sell() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let id = sv(&b, "id");
    clear();
    tq(182.0, 181.95);
    let b = get_bracket(&id);
    assert_eq!(sv(&b, "status"), "firing");
    assert_eq!(ops(), vec!["SoOrdersOrderCancel"], "the stop's cancel goes first, alone");
    assert_eq!(sv(&get_order(&sv(&b, "slOrderId")), "status"), "cancelling");
    clear();
    tq(182.0, 181.95);
    assert_eq!(ops(), vec!["FetchSoOrdersExtendedOrder"], "the stop is read back every check; nothing is written until Wealthsimple confirms the cancel");
    update_order(&sv(&b, "slOrderId"), json!({"status": "cancelled", "wsStatus": "CANCELLED"}));
    tq(182.0, 181.95);
    let b = get_bracket(&id);
    assert_eq!(sv(&b, "status"), "target_placed");
    let c = creates();
    assert_eq!((sv(&c[0], "executionType"), sv(&c[0], "orderType"), fv(&c[0], "limitPrice"), fv(&c[0], "quantity")), ("LIMIT".into(), "SELL_QUANTITY".into(), 181.94, 25.0));
    let tp = get_order(&sv(&b, "tpOrderId"));
    assert_eq!((sv(&tp, "role"), sv(&tp, "parentId")), ("target".into(), oid));
    update_order(&sv(&b, "tpOrderId"), json!({"status": "filled", "filledQty": 25, "avgFill": 181.94}));
    t0();
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), sv(&b, "outcome")), ("done".into(), "target".into()));
}

#[test]
fn test_the_bid_not_the_last_decides_the_target() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    clear();
    let id = sv(&b, "id");
    tq(182.0, 181.50);
    assert_eq!(sv(&get_bracket(&id), "status"), "armed");
    tq(181.0, 181.94);
    assert_eq!(sv(&get_bracket(&id), "status"), "firing");
}

#[test]
fn test_nothing_fires_while_the_market_is_closed() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    clear();
    tick(Some(q(190.0, None, "CLOSED")));
    assert!(sent_empty());
    assert_eq!(sv(&get_bracket(&sv(&b, "id")), "status"), "armed");
}

#[test]
fn test_the_stop_filling_ends_the_bracket() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let b = get_bracket(&sv(&b, "id"));
    update_order(&sv(&b, "slOrderId"), json!({"status": "filled", "filledQty": 25, "avgFill": 157.0}));
    t0();
    let b = get_bracket(&sv(&b, "id"));
    assert_eq!((sv(&b, "status"), sv(&b, "outcome")), ("done".into(), "stopped".into()));
}

#[test]
fn test_a_stop_cancelled_by_hand_ends_the_bracket() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let b = get_bracket(&sv(&b, "id"));
    update_order(&sv(&b, "slOrderId"), json!({"status": "cancelled"}));
    clear();
    t0();
    let b = get_bracket(&sv(&b, "id"));
    assert_eq!((sv(&b, "status"), sv(&b, "outcome")), ("done".into(), "stop cancelled at Wealthsimple by hand".into()));
    assert!(sent_empty(), "nothing else of the bracket's rested, so nothing to cancel");
    tick(Some(q(182.0, None, "OPEN")));
    assert_eq!(sv(&get_bracket(&sv(&b, "id")), "status"), "done", "the target never fires");
    assert!(ops().is_empty());
}

#[test]
fn test_while_the_limit_sell_rests_the_stop_level_is_watched_and_swaps_it_for_a_market_sell() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let id = sv(&b, "id");
    let b = get_bracket(&id);
    let stop = sv(&b, "slOrderId");
    tq(182.0, 182.0);
    update_order(&stop, json!({"status": "cancelled"}));
    tq(182.0, 182.0);
    let b = get_bracket(&id);
    assert_eq!(sv(&b, "status"), "target_placed");
    let limit = sv(&b, "tpOrderId");
    clear();
    tq(181.0, 181.0);
    assert!(sent_empty());
    tq(157.0, 157.0);
    assert_eq!(cancels(), vec![limit.clone()]);
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), sv(&b, "tpOrderId")), ("stopping".into(), "".into()));
    clear();
    tq(156.0, 156.0);
    assert!(creates().is_empty());
    update_order(&limit, json!({"status": "cancelled"}));
    tq(156.0, 156.0);
    let c = creates();
    assert_eq!((sv(&c[0], "executionType"), sv(&c[0], "orderType"), fv(&c[0], "quantity")), ("MARKET".into(), "SELL_QUANTITY".into(), 25.0), "a market sell, the stop having been hit");
    let b = get_bracket(&id);
    assert_eq!(sv(&b, "status"), "firing");
    filled(&sv(&b, "slOrderId"));
    t0();
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), sv(&b, "outcome")), ("done".into(), "stopped".into()));
    let pending = list_orders().iter().filter(|o| matches!(sv(o, "role").as_str(), "stop" | "target") && sv(o, "status") == "pending").count();
    assert_eq!(pending, 0, "nothing of the bracket's rests");
}

#[test]
fn test_the_limit_sell_gives_way_to_the_stop_order_once_the_target_is_out_of_reach() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let id = sv(&b, "id");
    let stop = sv(&get_bracket(&id), "slOrderId");
    tq(182.0, 182.0);
    update_order(&stop, json!({"status": "cancelled"}));
    tq(182.0, 182.0);
    let limit = sv(&get_bracket(&id), "tpOrderId");
    clear();
    tq(180.5, 180.5);
    assert!(sent_empty());
    tq(179.0, 179.0);
    assert_eq!(cancels(), vec![limit.clone()]);
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), sv(&b, "tpOrderId"), sv(&b, "slOrderId")), ("armed".into(), "".into(), "".into()));
    clear();
    tq(179.0, 179.0);
    assert!(creates().is_empty());
    update_order(&limit, json!({"status": "cancelled"}));
    tq(179.0, 179.0);
    let c = creates();
    assert_eq!((sv(&c[0], "executionType"), fv(&c[0], "stopPrice"), sv(&c[0], "timeInForce")), ("STOP".into(), 157.13, "UNTIL_CANCEL".into()), "the stop order rests at Wealthsimple again");
    let b = get_bracket(&id);
    assert_eq!(sv(&b, "status"), "armed");
    assert!(tv(&b, "slOrderId"));
    let targets: Vec<String> = list_orders().iter().filter(|o| sv(o, "role") == "target").map(|o| sv(o, "status")).collect();
    assert_eq!(targets, vec!["cancelled"], "no limit sell rests");
}

#[test]
fn test_a_trailing_level_keeps_following_the_high_while_the_limit_sell_rests() {
    let _g = setup();
    let (oid, b) = entry(json!({"stopLoss": {"kind": "trail", "trail": 5, "trailUnit": "pct"}, "takeProfit": {"price": 175.0}}));
    update_order(&oid, json!({"status": "filled", "filledQty": 25, "avgFill": 165.38}));
    t0();
    let id = sv(&b, "id");
    let stop = sv(&get_bracket(&id), "slOrderId");
    tq(175.5, 175.5);
    update_order(&stop, json!({"status": "cancelled"}));
    tq(175.5, 175.5);
    assert_eq!(sv(&get_bracket(&id), "status"), "target_placed");
    tq(180.0, 180.0);
    let b = get_bracket(&id);
    assert_eq!((fv(&b, "highWater"), fv(&b, "slPrice")), (180.0, 171.0), "the level follows the high with no order to move");
}

#[test]
fn test_a_price_changed_by_hand_at_wealthsimple_is_adopted() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let b = get_bracket(&sv(&b, "id"));
    update_order(&sv(&b, "slOrderId"), json!({"stopPrice": 160.0}));
    t0();
    assert_eq!(fv(&get_bracket(&sv(&b, "id")), "slPrice"), 160.0, "the bracket follows the level set by hand");
}

#[test]
fn test_an_ending_is_confirmed_before_the_bracket_is_done() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let id = sv(&b, "id");
    let stop = sv(&get_bracket(&id), "slOrderId");
    *lk(&bracket_seam::CANCEL_ORDER) = Some(json!({"ok": false, "error": "Wealthsimple is busy"}));
    od::cancel_bracket(&app(), &id);
    *lk(&bracket_seam::CANCEL_ORDER) = None;
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), sv(&b, "outcome")), ("closing".into(), "cancelled by the user".into()));
    assert!(["sent", "pending"].contains(&sv(&get_order(&stop), "status").as_str()), "the stop still rests: the cancel was refused");
    clear();
    t0();
    assert_eq!(ops(), vec!["SoOrdersOrderCancel"], "the cancel is sent again");
    assert_eq!(sv(&get_bracket(&id), "status"), "closing");
    update_order(&stop, json!({"status": "cancelled"}));
    t0();
    assert_eq!(sv(&get_bracket(&id), "status"), "done");
}

#[test]
fn test_an_exit_resting_with_no_bracket_holding_it_is_swept() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let id = sv(&b, "id");
    let stop = sv(&get_bracket(&id), "slOrderId");
    update_bracket(&id, json!({"status": "done", "outcome": "stopped", "slOrderId": ""}));
    clear();
    t0();
    assert_eq!(cancels(), vec![stop]);
}

#[test]
fn test_a_sell_from_the_ticket_ends_the_bracket_on_those_shares_first() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let id = sv(&b, "id");
    let stop = sv(&get_bracket(&id), "slOrderId");
    clear();
    let r = od::place_order(&app(), &ticket(json!({"side": "SELL", "stopLoss": null, "takeProfit": null})));
    assert!(tv(&r, "ok"), "{}", r);
    let o = ops();
    assert_eq!(o[0], "SoOrdersOrderCancel", "the resting stop goes first");
    assert_eq!(o[o.len() - 1], "SoOrdersOrderCreate", "then the sell");
    assert!(o.iter().position(|x| x == "SoOrdersOrderCancel") < o.iter().position(|x| x == "SoOrdersOrderCreate"));
    assert_eq!(sv(&get_bracket(&id), "outcome"), "sold from the ticket");
    assert!(["closing", "done"].contains(&sv(&get_bracket(&id), "status").as_str()));
    assert_eq!(sv(&get_order(&stop), "status"), "cancelling");
}

#[test]
fn test_selling_part_of_the_shares_keeps_the_bracket_on_the_rest() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let id = sv(&b, "id");
    let first = sv(&get_bracket(&id), "slOrderId");
    let r = od::place_order(&app(), &ticket(json!({"side": "SELL", "quantity": 10, "stopLoss": null, "takeProfit": null})));
    assert!(tv(&r, "ok"), "{}", r);
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), fv(&b, "quantity"), sv(&b, "slOrderId")), ("armed".into(), 15.0, "".into()));
    update_order(&first, json!({"status": "cancelled"}));
    clear();
    t0();
    let c = creates();
    assert_eq!((sv(&c[0], "executionType"), fv(&c[0], "quantity"), fv(&c[0], "stopPrice")), ("STOP".into(), 15.0, 157.13), "a stop on the fifteen left");
}

#[test]
fn test_a_watched_stop_fires_as_a_market_sell_when_wealthsimple_takes_no_stop_order() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    let id = sv(&b, "id");
    *lk(&bracket_seam::STOP_ALLOWED) = Some(false);
    t0();
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), sv(&b, "slMode"), sv(&b, "slOrderId")), ("armed".into(), "watched".into(), "".into()));
    clear();
    tq(158.0, 157.90);
    assert!(sent_empty());
    tq(157.2, 157.10);
    *lk(&bracket_seam::STOP_ALLOWED) = None;
    let b = get_bracket(&id);
    assert_eq!(sv(&b, "status"), "firing");
    let c = creates();
    assert_eq!((sv(&c[0], "executionType"), sv(&c[0], "orderType")), ("MARKET".into(), "SELL_QUANTITY".into()));
    assert!(c[0].get("stopPrice").is_none());
    filled(&sv(&b, "slOrderId"));
    t0();
    assert_eq!(sv(&get_bracket(&id), "outcome"), "stopped");
}

#[test]
fn test_a_trailing_stop_follows_the_high_by_cancel_and_replace_no_more_than_once_a_minute() {
    let _g = setup();
    let (oid, b) = entry(json!({"stopLoss": {"kind": "trail", "trail": 5, "trailUnit": "pct"}}));
    update_order(&oid, json!({"status": "filled", "filledQty": 25, "avgFill": 165.4}));
    t0();
    let id = sv(&b, "id");
    let b = get_bracket(&id);
    let first_stop = sv(&b, "slOrderId");
    assert_eq!(sv(&b, "slKind"), "trail");
    clear();
    tick(Some(q(180.0, None, "OPEN")));
    let b = get_bracket(&id);
    assert_eq!(fv(&b, "highWater"), 180.0);
    assert_eq!(fv(&b, "slPrice"), 171.0, "the high less five percent");
    assert_eq!(ops(), vec!["SoOrdersOrderCancel"], "the old stop is cancelled; the new one waits for the confirmation");
    assert_eq!(sv(&b, "slOrderId"), "");
    update_order(&first_stop, json!({"status": "cancelled"}));
    clear();
    tick(Some(q(180.0, None, "OPEN")));
    let b = get_bracket(&id);
    let c = creates();
    assert_eq!((sv(&c[0], "executionType"), fv(&c[0], "stopPrice")), ("STOP".into(), 171.0));
    assert_ne!(sv(&b, "slOrderId"), first_stop);
    clear();
    tick(Some(q(180.5, None, "OPEN")));
    assert!(sent_empty());
    assert_eq!(fv(&get_bracket(&id), "slPrice"), 171.0);
    tick(Some(q(171.5, None, "OPEN")));
    assert_eq!(fv(&get_bracket(&id), "slPrice"), 171.0, "a lower high never lowers the stop");
    tick(Some(q(181.5, None, "OPEN")));
    assert_eq!(fv(&get_bracket(&id), "slPrice"), 172.43);
}

#[test]
fn test_the_balances_feed_never_touches_a_bracket_with_a_resting_order() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let id = sv(&b, "id");
    replace_balances(json!([{"accountId": "acct-margin", "securityId": "sec-other", "quantity": 1}]));
    update_bracket(&id, json!({"armedAt": "2020-01-01T00:00:00Z", "seenHeld": true}));
    clear();
    for stamp in ["2099-01-01T00:00:00Z", "2099-01-01T01:00:00Z", "2099-01-01T02:00:00Z"] {
        set_meta("balances_read_at", stamp);
        t0();
    }
    let b = get_bracket(&id);
    assert_eq!(sv(&b, "status"), "armed", "reads without the position change nothing while the stop rests");
    assert!(tv(&b, "slOrderId"), "the stop still rests");
    assert!(sent_empty(), "nothing is cancelled on the balances' word");
}

fn watched_only() -> (String, Value) {
    let (oid, b) = entry(json!({}));
    filled(&oid);
    *lk(&bracket_seam::STOP_ALLOWED) = Some(false);
    t0();
    *lk(&bracket_seam::STOP_ALLOWED) = None;
    let b = get_bracket(&sv(&b, "id"));
    assert_eq!((sv(&b, "status"), sv(&b, "slMode"), sv(&b, "slOrderId")), ("armed".into(), "watched".into(), "".into()));
    update_bracket(&sv(&b, "id"), json!({"armedAt": "2020-01-01T00:00:00Z"}));
    (oid, b)
}

#[test]
fn test_a_watched_only_bracket_ends_on_the_second_balance_read_without_the_position() {
    let _g = setup();
    let (_oid, b) = watched_only();
    let id = sv(&b, "id");
    replace_balances(json!([{"accountId": "acct-margin", "securityId": "sec-s-us", "quantity": 25}]));
    set_meta("balances_read_at", "2099-01-01T00:00:00Z");
    *lk(&bracket_seam::STOP_ALLOWED) = Some(false);
    t0();
    assert!(tv(&get_bracket(&id), "seenHeld"));
    replace_balances(json!([{"accountId": "acct-margin", "securityId": "sec-other", "quantity": 1}]));
    set_meta("balances_read_at", "2099-01-01T01:00:00Z");
    t0();
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), sv(&b, "missedAt")), ("armed".into(), "2099-01-01T01:00:00Z".into()), "one read never ends it");
    t0();
    assert_eq!(sv(&get_bracket(&id), "status"), "armed", "the same read again is still one read");
    set_meta("balances_read_at", "2099-01-01T02:00:00Z");
    t0();
    *lk(&bracket_seam::STOP_ALLOWED) = None;
    let b = get_bracket(&id);
    assert_eq!(sv(&b, "status"), "done");
    assert!(sv(&b, "outcome").contains("two balance reads"), "{}", sv(&b, "outcome"));
}

#[test]
fn test_a_watched_only_bracket_ends_when_the_activity_feed_shows_the_sale() {
    let _g = setup();
    let (_oid, b) = watched_only();
    let act = json!({"id": "act-sale", "transactionDate": "2026-09-10", "occurredAt": "2026-09-10T15:00:00Z", "accountId": "acct-margin", "securityId": "sec-s-us", "symbol": "QNC",
        "quantity": 25, "unitPrice": 170.0, "netCashAmount": 4250.0, "activityType": "Trade", "activitySubType": "SELL", "source": "csv"});
    bagholder_store::activities::insert_activity(&db(), &act, None, None, &crate::app::uuid4).unwrap();
    *lk(&bracket_seam::STOP_ALLOWED) = Some(false);
    t0();
    *lk(&bracket_seam::STOP_ALLOWED) = None;
    let b = get_bracket(&sv(&b, "id"));
    assert_eq!(sv(&b, "status"), "done");
    assert!(sv(&b, "outcome").contains("sold"), "{}", sv(&b, "outcome"));
}

#[test]
fn test_a_rejected_exit_is_tried_again_spaced_out_for_as_long_as_the_bracket_lives() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    *lk(&REJECTIONS) = vec!["Market closed".to_string(); 6];
    t0();
    let id = sv(&b, "id");
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), fv(&b, "attempts")), ("armed".into(), 1.0));
    assert!(sv(&b, "error").contains("Market closed"));
    clear();
    t0();
    assert!(creates().is_empty(), "no second try within the minute");
    for n in 2..7 {
        so::update_bracket(&db(), &id, &json!({"error": "Market closed"}), "2020-01-01T00:00:00Z").unwrap();
        t0();
        let b = get_bracket(&id);
        assert_eq!((sv(&b, "status"), fv(&b, "attempts")), ("armed".into(), n as f64), "attempt {}, still armed", n);
    }
    so::update_bracket(&db(), &id, &json!({"error": "Market closed"}), "2020-01-01T00:00:00Z").unwrap();
    t0();
    let b = get_bracket(&id);
    assert!(tv(&b, "slOrderId"));
    assert_eq!((sv(&b, "status"), fv(&b, "attempts"), sv(&b, "error")), ("armed".into(), 0.0, "".into()));
}

#[test]
fn test_with_orders_off_nothing_is_placed_and_the_line_is_printed_once() {
    let _g = setup();
    live(false);
    let r = od::place_order(&app(), &ticket(json!({})));
    let (oid, bid) = (sv(&r, "id"), sv(&r, "bracketId"));
    filled(&oid);
    od::bracket_tick(&app(), Some(HashMap::new()));
    od::bracket_tick(&app(), Some(HashMap::new()));
    let b = get_bracket(&bid);
    assert_eq!((sv(&b, "status"), sv(&b, "slOrderId")), ("armed".into(), "".into()));
    assert!(creates().is_empty());
    let n = lk(&bracket_seam::SAID).iter().map(|l| l.matches("orders are off, not placed").count()).sum::<usize>();
    assert_eq!(n, 1);
}

#[test]
fn test_cancel_bracket_cancels_its_resting_orders_and_stops_watching() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let id = sv(&b, "id");
    clear();
    let r = od::cancel_bracket(&app(), &id);
    assert!(tv(&r, "ok"));
    assert_eq!(ops(), vec!["SoOrdersOrderCancel"]);
    let b = get_bracket(&id);
    assert_eq!((sv(&b, "status"), sv(&b, "outcome")), ("closing".into(), "cancelled by the user".into()));
    assert_eq!(sv(&get_order(&oid), "status"), "filled", "the entry is not touched");
    for o in list_orders() {
        if sv(&o, "role") == "stop" {
            update_order(&sv(&o, "id"), json!({"status": "cancelled"}));
        }
    }
    t0();
    let b = get_bracket(&id);
    assert_eq!(sv(&b, "status"), "done", "done once Wealthsimple confirms the cancel");
    assert!(sv(&od::cancel_bracket(&app(), &id), "error").contains("not live"));
    assert_eq!(sv(&od::cancel_bracket(&app(), "nope"), "error"), "No such bracket.");
    let ids: Vec<String> = od::orders_payload(&app(), false)["brackets"].as_array().unwrap().iter().map(|x| sv(x, "id")).collect();
    assert_eq!(ids, vec![id]);
}

// --- a resting exit is placed again before Wealthsimple lets it lapse ---

/// The time `days` from now, as Wealthsimple writes one.
fn in_days(days: f64) -> String {
    let t = (crate::app::now_unix() + days * 86400.0) as i64;
    let (y, m, d) = bagholder_model::dates::from_days(t.div_euclid(86400));
    let s = t.rem_euclid(86400);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z", y, m, d, s / 3600, s % 3600 / 60, s % 60)
}

/// An armed bracket whose stop rests at Wealthsimple, lapsing in `days`.
fn armed_with_stop_lapsing_in(days: f64) -> (Value, String) {
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let b = get_bracket(&sv(&b, "id"));
    let stop = sv(&b, "slOrderId");
    update_order(&stop, json!({"expiresAt": in_days(days)}));
    clear();
    (b, stop)
}

#[test]
fn test_a_stop_far_from_lapsing_is_left_alone() {
    let _g = setup();
    let (b, stop) = armed_with_stop_lapsing_in(30.0);
    tick(Some(q(170.0, None, "CLOSED")));
    tq(170.0, 170.0);
    assert!(ops().is_empty(), "{:?}", ops());
    assert_eq!(sv(&get_bracket(&sv(&b, "id")), "slOrderId"), stop);
}

#[test]
fn test_a_stop_within_a_week_of_lapsing_is_rolled_while_the_market_is_closed_not_while_it_trades() {
    let _g = setup();
    let (b, stop) = armed_with_stop_lapsing_in(5.0);
    tq(170.0, 170.0);
    assert!(ops().is_empty(), "with days in hand, the stop is not taken off a trading market: {:?}", ops());
    tick(Some(q(170.0, None, "CLOSED")));
    assert_eq!(cancels(), vec![stop.clone()]);
    let after = get_bracket(&sv(&b, "id"));
    assert_eq!((sv(&after, "status"), sv(&after, "slOrderId"), fv(&after, "slPrice")), ("armed".into(), "".into(), 157.13));
    assert!(creates().is_empty(), "not placed again while the old one's cancel is still in the air");
    // Wealthsimple confirms the cancel: the stop goes back at the same level, and the bracket stands
    update_order(&stop, json!({"status": "cancelled"}));
    clear();
    tick(Some(q(170.0, None, "CLOSED")));
    let c = creates();
    assert_eq!(c.len(), 1, "{:?}", ops());
    assert_eq!((sv(&c[0], "executionType"), c[0]["stopPrice"].as_f64()), ("STOP".into(), Some(157.13)));
    let again = get_bracket(&sv(&b, "id"));
    assert_eq!(sv(&again, "status"), "armed");
    assert!(sv(&again, "slOrderId").starts_with("order-") && sv(&again, "slOrderId") != stop);
}

#[test]
fn test_a_stop_within_two_days_of_lapsing_is_rolled_whatever_the_market_is_doing() {
    let _g = setup();
    let (_b, stop) = armed_with_stop_lapsing_in(1.5);
    tq(170.0, 170.0);
    assert_eq!(cancels(), vec![stop]);
}

#[test]
fn test_a_good_till_cancelled_stop_with_no_expiry_given_lapses_ninety_days_from_when_it_was_sent() {
    let _g = setup();
    let (oid, b) = entry(json!({}));
    filled(&oid);
    t0();
    let stop = sv(&get_bracket(&sv(&b, "id")), "slOrderId");
    // sent eighty-nine days ago, and Wealthsimple has not said when it lapses
    update_order(&stop, json!({"submittedAt": in_days(-89.0), "expiresAt": ""}));
    clear();
    tq(170.0, 170.0);
    assert_eq!(cancels(), vec![stop]);
}

#[test]
fn test_a_resting_target_is_rolled_too_and_placed_again() {
    let _g = setup();
    let (oid, b) = entry(json!({"stopLoss": null}));
    filled(&oid);
    t0();
    tq(182.0, 182.0);
    let placed = get_bracket(&sv(&b, "id"));
    assert_eq!(sv(&placed, "status"), "target_placed");
    let target = sv(&placed, "tpOrderId");
    update_order(&target, json!({"expiresAt": in_days(1.0)}));
    clear();
    tq(181.0, 181.0);
    assert_eq!(cancels(), vec![target.clone()]);
    assert_eq!(sv(&get_bracket(&sv(&b, "id")), "tpOrderId"), "");
    update_order(&target, json!({"status": "cancelled"}));
    clear();
    tq(181.0, 181.0);
    let c = creates();
    assert_eq!(c.len(), 1, "{:?}", ops());
    assert_eq!((sv(&c[0], "executionType"), c[0]["limitPrice"].as_f64()), ("LIMIT".into(), Some(181.94)));
    assert_eq!(sv(&get_bracket(&sv(&b, "id")), "status"), "target_placed");
}
