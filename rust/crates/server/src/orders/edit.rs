//! Changing a resting order, and a leg of a bracket.

use super::*;

pub fn modify_order(order_id: &str, quantity: Option<&Value>, limit_price: Option<&Value>) -> Value {
    let row = match get_order(order_id) {
        Some(r) => r,
        None => return json!({"ok": false, "error": "No such order."}),
    };
    if !st_in(&row, &["sent", "pending"]) {
        return json!({"ok": false, "error": "That order is not open."});
    }
    let typ = f(&row, "type");
    if typ == "STOP" {
        return json!({"ok": false, "error": "A stop order cannot be changed; cancel it and place another."});
    }
    let q = num(quantity, None);
    let lp = order_tick(num(limit_price, None));
    if q.map_or(false, |x| x <= 0.0) {
        return json!({"ok": false, "error": "Shares must be more than zero."});
    }
    if lp.map_or(false, |x| x <= 0.0) {
        return json!({"ok": false, "error": "A limit price must be more than zero."});
    }
    let limit_type = typ == "LIMIT" || typ == "STOP_LIMIT";
    if limit_type && lp.is_none() && q.is_none() {
        return json!({"ok": false, "error": "Nothing to change."});
    }
    let id = f(&row, "id");
    let mut inp = json!({"externalId": id});
    if lp.is_some() && limit_type && lp != on(&row, "limitPrice") {
        set(&mut inp, "newLimitPrice", jo(lp));
    }
    if q.is_some() && q != on(&row, "quantity") {
        set(&mut inp, "newQuantity", jo(q));
    }
    let inp_map = inp.as_object().cloned().unwrap_or_default();
    if inp_map.len() == 1 {
        return json!({"ok": true, "id": id, "unchanged": true});
    }
    if !orders_live() {
        return json!({"ok": false, "error": "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple."});
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "error": "Not connected."}),
    };
    let data = match gql(&sess, "SoOrdersOrderModify", json!({"input": inp})) {
        Ok(d) => d,
        Err(CallError::NotAuthorized) => return json!({"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}),
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
    let mut patch = Map::new();
    if inp_map.contains_key("newLimitPrice") {
        patch.insert("limitPrice".into(), jo(lp));
    }
    if inp_map.contains_key("newQuantity") {
        patch.insert("quantity".into(), jo(q));
    }
    update_order(&id, Value::Object(patch));
    // a bracket still waiting on this entry guards the size the entry now has
    if let Some(b) = must(so::typed::bracket_for_order(&db(), &id)).filter(|b| b.status == BracketStatus::Waiting && inp_map.contains_key("newQuantity")) {
        patch_bracket(&b.id, BracketPatch { quantity: Some(q), ..BracketPatch::default() });
    }
    let shown: Map<String, Value> = inp_map.iter().filter(|(k, _)| *k != "externalId").map(|(k, v)| (k.clone(), v.clone())).collect();
    log(&format!("bagholder orders: modify {} accepted: {}", id, bagholder_store::tables::json_text_sorted(&Value::Object(shown))));
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
