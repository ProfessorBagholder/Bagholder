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
    if let Some(b) = must(so::bracket_for_order(&db(), &id)) {
        if f(&b, "status") == "waiting" && inp_map.contains_key("newQuantity") {
            update_bracket(&f(&b, "id"), json!({"quantity": jo(q)}));
        }
    }
    let shown: Map<String, Value> = inp_map.iter().filter(|(k, _)| *k != "externalId").map(|(k, v)| (k.clone(), v.clone())).collect();
    log(&format!("bagholder orders: modify {} accepted: {}", id, bagholder_store::tables::json_text_sorted(&Value::Object(shown))));
    let rid = id.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(|| refresh_orders(&rid));
    });
    json!({"ok": true, "id": id})
}

pub fn adjust_bracket(bracket_id: &str, leg: &str, price: Option<&Value>, trail: Option<&Value>, remove: bool) -> Value {
    let b = match get_bracket(bracket_id) {
        Some(b) => b,
        None => return json!({"ok": false, "error": "No such bracket."}),
    };
    if !st_in(&b, &BRACKET_LIVE) {
        return json!({"ok": false, "error": "That bracket is not live."});
    }
    let leg = leg.to_lowercase();
    if leg != "sl" && leg != "tp" {
        return json!({"ok": false, "error": "Which leg?"});
    }
    let id = f(&b, "id");
    let sym = f(&b, "symbol");
    if remove {
        if leg == "sl" {
            let err = cancel_exit(&f(&b, "slOrderId"));
            if !err.is_empty() {
                return json!({"ok": false, "error": err});
            }
            let mut patch = json!({"slKind": "", "slOrderId": "", "slMode": "", "error": ""});
            if !tr(&b, "tpPrice") {
                set(&mut patch, "status", json!("cancelled"));
                set(&mut patch, "outcome", json!("both legs removed"));
            }
            update_bracket(&id, patch);
        } else {
            let err = cancel_exit(&f(&b, "tpOrderId"));
            if !err.is_empty() {
                return json!({"ok": false, "error": err});
            }
            let mut patch = json!({"tpPrice": null, "tpOrderId": "", "error": ""});
            if f(&b, "status") == "target_placed" {
                set(&mut patch, "status", json!("armed"));
            }
            if !tr(&b, "slKind") {
                set(&mut patch, "status", json!("cancelled"));
                set(&mut patch, "outcome", json!("both legs removed"));
            }
            update_bracket(&id, patch);
        }
        log(&format!("bagholder bracket: {} for {}: {} removed by the user", id, sym, if leg == "sl" { "stop loss" } else { "take profit" }));
        return json!({"ok": true, "id": id});
    }
    if leg == "sl" {
        if !tr(&b, "slKind") {
            return json!({"ok": false, "error": "This bracket has no stop loss."});
        }
        let mut patch;
        if f(&b, "slKind") == "trail" {
            let t = match num(trail, None) {
                Some(t) if t != 0.0 && t > 0.0 => t,
                _ => return json!({"ok": false, "error": "A trail is required."}),
            };
            let high = or_f(or_f(on(&b, "highWater"), on(&b, "slPrice")), Some(0.0)).unwrap_or(0.0);
            let mut nb = b.clone();
            set(&mut nb, "slTrail", json!(t));
            let new_price = if high != 0.0 { json!(round_half_even(high - trail_distance(&nb, high).unwrap_or(0.0), 2)) } else { gv(&b, "slPrice") };
            patch = json!({"slTrail": t, "slPrice": new_price});
        } else {
            let p = match num(price, None) {
                Some(p) if p != 0.0 && p > 0.0 => p,
                _ => return json!({"ok": false, "error": "A stop price is required."}),
            };
            patch = json!({"slPrice": p});
        }
        if tr(&b, "slOrderId") && f(&b, "status") == "armed" {
            let err = cancel_exit(&f(&b, "slOrderId"));
            if !err.is_empty() {
                return json!({"ok": false, "error": err});
            }
            set(&mut patch, "slOrderId", json!(""));
            set(&mut patch, "movedAt", json!(now_iso()));
        }
        set(&mut patch, "error", json!(""));
        let shown = rp(on(&patch, "slPrice"));
        update_bracket(&id, patch);
        log(&format!("bagholder bracket: {} for {}: stop moved to {} by the user", id, sym, shown));
        return json!({"ok": true, "id": id});
    }
    let p = match num(price, None) {
        Some(p) if p != 0.0 && p > 0.0 => p,
        _ => return json!({"ok": false, "error": "A limit price is required."}),
    };
    let mut patch = json!({"tpPrice": p, "error": ""});
    if f(&b, "status") == "target_placed" && tr(&b, "tpOrderId") {
        let err = cancel_exit(&f(&b, "tpOrderId"));
        if !err.is_empty() {
            return json!({"ok": false, "error": err});
        }
        set(&mut patch, "tpOrderId", json!(""));
        set(&mut patch, "status", json!("armed"));
    }
    update_bracket(&id, patch);
    log(&format!("bagholder bracket: {} for {}: target moved to {} by the user", id, sym, rp(Some(p))));
    json!({"ok": true, "id": id})
}
