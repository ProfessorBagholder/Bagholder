//! Changing a resting order, and a leg of a bracket.

use super::*;

/// Change a resting order's size or its limit. A stop order cannot be changed at
/// Wealthsimple, only cancelled and placed again.
pub fn modify_order(order_id: &str, quantity: Option<&Value>, limit_price: Option<&Value>) -> Value {
    let refused = |e: &str| json!({"ok": false, "error": e});
    let row = match order(order_id) {
        Some(r) => r,
        None => return refused("No such order."),
    };
    if !matches!(row.status, OrderStatus::Sent | OrderStatus::Pending) {
        return refused("That order is not open.");
    }
    if row.kind == OrderType::Stop {
        return refused("A stop order cannot be changed; cancel it and place another.");
    }
    let q = num(quantity, None);
    let lp = order_tick(num(limit_price, None));
    if q.map_or(false, |x| x <= 0.0) {
        return refused("Shares must be more than zero.");
    }
    if lp.map_or(false, |x| x <= 0.0) {
        return refused("A limit price must be more than zero.");
    }
    let limit_type = matches!(row.kind, OrderType::Limit | OrderType::StopLimit);
    if limit_type && lp.is_none() && q.is_none() {
        return refused("Nothing to change.");
    }
    let id = row.id.clone();
    // only what differs from the order as it rests is asked for
    let new_limit = lp.filter(|_| limit_type && lp != row.limit_price);
    let new_quantity = q.filter(|_| q != row.quantity);
    if new_limit.is_none() && new_quantity.is_none() {
        return json!({"ok": true, "id": id, "unchanged": true});
    }
    if !orders_live() {
        return refused("Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple.");
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return refused("Not connected."),
    };
    let mut change = Map::new();
    if let Some(p) = new_limit {
        change.insert("newLimitPrice".into(), json!(p));
    }
    if let Some(n) = new_quantity {
        change.insert("newQuantity".into(), json!(n));
    }
    let mut inp = change.clone();
    inp.insert("externalId".into(), json!(id));
    let data = match gql(&sess, "SoOrdersOrderModify", json!({"input": inp})) {
        Ok(d) => d,
        Err(CallError::NotAuthorized) => return refused("Wealthsimple refused the session. Connect Wealthsimple again."),
        Err(e) => {
            let msg = err_text(&e);
            log(&format!("bagholder orders: modify {} failed: {}", id, msg));
            return json!({"ok": false, "error": format!("Change failed: {}", msg)});
        }
    };
    if let Some(msg) = data.get("soOrdersModifyOrder").and_then(|r| r.get("errors")).filter(|v| truthy(Some(v))).and_then(first_error) {
        log(&format!("bagholder orders: modify {} refused: {}", id, msg));
        return json!({"ok": false, "error": format!("Wealthsimple refused the change: {}", msg)});
    }
    patch_order(&id, OrderPatch { limit_price: new_limit.map(Some), quantity: new_quantity.map(Some), ..OrderPatch::default() });
    // a bracket still waiting on this entry guards the size the entry now has
    if let Some(b) = must(so::typed::bracket_for_order(&db(), &id)).filter(|b| b.status == BracketStatus::Waiting && new_quantity.is_some()) {
        patch_bracket(&b.id, BracketPatch { quantity: Some(new_quantity), ..BracketPatch::default() });
    }
    log(&format!("bagholder orders: modify {} accepted: {}", id, bagholder_store::tables::json_text_sorted(&Value::Object(change))));
    let rid = id.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(|| refresh_orders(&rid));
    });
    json!({"ok": true, "id": id})
}

/// Move a leg of a live bracket, give a trailing stop another trail, or take a leg off.
/// A leg that rests at Wealthsimple is cancelled first, and the engine places it again
/// at the new level; a bracket left with no leg is over.
pub fn adjust_bracket(bracket_id: &str, leg: &str, price: Option<&Value>, trail: Option<&Value>, remove: bool) -> Value {
    let refused = |e: String| json!({"ok": false, "error": e});
    let b = match bracket(bracket_id) {
        Some(b) => b,
        None => return refused("No such bracket.".into()),
    };
    if !b.status.is_live() {
        return refused("That bracket is not live.".into());
    }
    let stop_leg = match leg.to_lowercase().as_str() {
        "sl" => true,
        "tp" => false,
        _ => return refused("Which leg?".into()),
    };
    let positive = |v: Option<&Value>| num(v, None).filter(|x| *x > 0.0);
    let both_removed = |p: &mut BracketPatch| {
        p.status = Some(BracketStatus::Cancelled);
        p.outcome = Some("both legs removed".into());
    };
    let mut patch = BracketPatch { error: Some(String::new()), ..BracketPatch::default() };
    if remove {
        let err = cancel_exit(if stop_leg { &b.sl_order_id } else { &b.tp_order_id });
        if !err.is_empty() {
            return refused(err);
        }
        if stop_leg {
            patch.sl_kind = Some(SlKind::Unset);
            patch.sl_order_id = Some(String::new());
            patch.sl_mode = Some(SlMode::Unset);
            if !some(b.tp_price) {
                both_removed(&mut patch);
            }
        } else {
            patch.tp_price = Some(None);
            patch.tp_order_id = Some(String::new());
            if b.status == BracketStatus::TargetPlaced {
                patch.status = Some(BracketStatus::Armed);
            }
            if !b.sl_kind.is_set() {
                both_removed(&mut patch);
            }
        }
        patch_bracket(&b.id, patch);
        log(&format!("bagholder bracket: {} for {}: {} removed by the user", b.id, b.symbol, if stop_leg { "stop loss" } else { "take profit" }));
        return json!({"ok": true, "id": b.id});
    }
    if stop_leg {
        if !b.sl_kind.is_set() {
            return refused("This bracket has no stop loss.".into());
        }
        let new_price;
        if b.sl_kind == SlKind::Trail {
            let Some(t) = positive(trail) else { return refused("A trail is required.".into()) };
            let high = or_f(or_f(b.high_water, b.sl_price), Some(0.0)).unwrap_or(0.0);
            let retrailed = Bracket { sl_trail: Some(t), ..b.clone() };
            new_price = if high != 0.0 { Some(round_half_even(high - trail_distance(&retrailed, high).unwrap_or(0.0), 2)) } else { b.sl_price };
            patch.sl_trail = Some(Some(t));
        } else {
            let Some(p) = positive(price) else { return refused("A stop price is required.".into()) };
            new_price = Some(p);
        }
        patch.sl_price = Some(new_price);
        if !b.sl_order_id.is_empty() && b.status == BracketStatus::Armed {
            let err = cancel_exit(&b.sl_order_id);
            if !err.is_empty() {
                return refused(err);
            }
            patch.sl_order_id = Some(String::new());
            patch.moved_at = Some(now_iso());
        }
        patch_bracket(&b.id, patch);
        log(&format!("bagholder bracket: {} for {}: stop moved to {} by the user", b.id, b.symbol, rp(new_price)));
        return json!({"ok": true, "id": b.id});
    }
    let Some(p) = positive(price) else { return refused("A limit price is required.".into()) };
    patch.tp_price = Some(Some(p));
    if b.status == BracketStatus::TargetPlaced && !b.tp_order_id.is_empty() {
        let err = cancel_exit(&b.tp_order_id);
        if !err.is_empty() {
            return refused(err);
        }
        patch.tp_order_id = Some(String::new());
        patch.status = Some(BracketStatus::Armed);
    }
    patch_bracket(&b.id, patch);
    log(&format!("bagholder bracket: {} for {}: target moved to {} by the user", b.id, b.symbol, rp(Some(p))));
    json!({"ok": true, "id": b.id})
}
