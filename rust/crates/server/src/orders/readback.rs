//! Reading orders back from Wealthsimple: what became of each, the fills booked from
//! them, cancelling one, and what the Orders panel is sent.

use super::*;

// --- reading orders back ---

pub(super) const ORDER_BRANCH: &str = "TR";
pub const WS_PENDING: [&str; 8] = ["NEW", "PENDING_SUBMISSION", "PENDING_REVIEW", "PENDING_FUND_TRANSFER", "SUBMITTED", "PLACED", "PARTIALLY_FILLED", "CONTINGENT"];
pub(super) const WS_CANCELLING: [&str; 1] = ["CANCEL_PENDING"];
pub const LIVE_STATUSES: [&str; 3] = ["sent", "pending", "cancelling"];
pub const ORDERS_REFRESH_SEC: u64 = 30;

pub(super) fn ws_status_map(s: &str) -> Option<&'static str> {
    Some(match s {
        "FILLED" | "POSTED" => "filled",
        "CANCELLED" | "DELETED" => "cancelled",
        "EXPIRED" => "expired",
        "REJECTED" => "rejected",
        _ => return None,
    })
}

pub(super) fn is_live(o: &Value) -> bool {
    LIVE_STATUSES.contains(&f(o, "status").as_str())
}

pub(super) fn qty_words(q: Option<f64>) -> String {
    let q = or_f(q, Some(0.0)).unwrap_or(0.0);
    if q.fract() == 0.0 {
        format!("{}", q as i64)
    } else {
        fmt_g(q)
    }
}

pub(super) fn price_words(p: Option<f64>) -> String {
    match p {
        None => "—".into(),
        Some(p) => {
            if p.abs() < 1.0 && round_half_even(p, 3) != round_half_even(p, 2) {
                format!("{:.3}", p)
            } else {
                format!("{:.2}", p)
            }
        }
    }
}

pub(super) fn order_words(o: &Value) -> String {
    let side = if f(o, "side") == "BUY" { "Buy" } else { "Sell" };
    let how = match f(o, "type").as_str() {
        "MARKET" => "at market".to_string(),
        "STOP" => format!("stop {}", price_words(on(o, "stopPrice"))),
        "STOP_LIMIT" => format!("stop {} · limit {}", price_words(on(o, "stopPrice")), price_words(on(o, "limitPrice"))),
        _ => format!("at {} limit", price_words(on(o, "limitPrice"))),
    };
    format!("{} {} {}", side, qty_words(on(o, "quantity")), how)
}

/// (kind, key, title, body).
pub fn order_notice(before: &Value, upd: &Value) -> Option<(String, String, String, String)> {
    let (was, now) = (f(before, "status"), f(upd, "status"));
    let sym = if tr(before, "symbol") { f(before, "symbol") } else { "?".into() };
    let role = if tr(before, "role") { f(before, "role") } else { "entry".into() };
    let acct = f(before, "account");
    let tail = if acct.is_empty() { String::new() } else { format!(" · {}", acct) };
    let oid = f(before, "id");
    let qty = or_f(or_f(on(upd, "filledQty"), on(before, "filledQty")), num(before.get("quantity"), Some(0.0)));
    let px = or_f(on(upd, "avgFill"), on(before, "avgFill"));
    let at = match px {
        Some(p) if p != 0.0 => format!(" at {}", price_words(Some(p))),
        _ => String::new(),
    };
    if now == "filled" && was != "filled" {
        let did = if f(before, "side") == "SELL" { "Sold " } else { "Bought " };
        let head = match role.as_str() {
            "stop" => "Stopped out · ",
            "target" => "Target hit · ",
            _ => "Order filled · ",
        };
        return Some(("fills".into(), format!("order:{}:filled", oid), format!("{}{}", head, sym), format!("{}{}{}{}", did, qty_words(qty), at, tail)));
    }
    if (now == "rejected" || now == "failed") && was != "rejected" && was != "failed" {
        let reason = if tr(upd, "error") { f(upd, "error") } else { f(before, "error") };
        let title = format!("{}{}", if now == "rejected" { "Order rejected · " } else { "Order not sent · " }, sym);
        let body = format!("{}{}", order_words(before), if reason.is_empty() { tail.clone() } else { format!(" · {}", reason) });
        return Some(("problems".into(), format!("order:{}:{}", oid, now), title, body));
    }
    if role == "stop" || role == "target" {
        return None;
    }
    if now == "expired" && was != "expired" {
        return Some(("problems".into(), format!("order:{}:expired", oid), format!("Order expired · {}", sym), format!("{}{}", order_words(before), tail)));
    }
    if now == "cancelled" && was != "cancelled" && was != "cancelling" {
        return Some(("problems".into(), format!("order:{}:cancelled", oid), format!("Order cancelled · {}", sym), format!("{}{}", order_words(before), tail)));
    }
    let filled = or_f(num(upd.get("filledQty"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    let before_filled = or_f(num(before.get("filledQty"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    let before_qty = or_f(num(before.get("quantity"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    if LIVE_STATUSES.contains(&now.as_str()) && filled > before_filled && filled < before_qty {
        return Some((
            "fills".into(),
            format!("order:{}:partial:{}", oid, qty_words(Some(filled))),
            format!("Partly filled · {}", sym),
            format!("{} of {}{}{}", qty_words(Some(filled)), qty_words(on(before, "quantity")), at, tail),
        ));
    }
    None
}

pub fn app_status(ws_status: &str) -> String {
    let s = ws_status.to_uppercase();
    if WS_PENDING.contains(&s.as_str()) {
        return "pending".into();
    }
    if WS_CANCELLING.contains(&s.as_str()) {
        return "cancelling".into();
    }
    match ws_status_map(&s) {
        Some(x) => x.into(),
        None => if s.is_empty() { String::new() } else { "pending".into() },
    }
}

pub fn parse_extended_order(data: &Value) -> Option<Value> {
    let o = data.get("soOrdersExtendedOrder")?;
    if !o.is_object() || !tr(o, "status") {
        return None;
    }
    Some(json!({
        "wsStatus": f(o, "status").to_uppercase(),
        "status": app_status(&f(o, "status")),
        "filledQty": jo(on(o, "filledQuantity")),
        "avgFill": jo(on(o, "averageFilledPrice")),
        "submittedAt": f(o, "submittedAtUtc"),
        "expiresAt": f(o, "expiredAtUtc"),
        "firstFilledAt": f(o, "firstFilledAtUtc"),
        "lastFilledAt": f(o, "lastFilledAtUtc"),
        "error": s(or_v(o.get("rejectionCause"), o.get("rejectionCode"))),
        "quantity": jo(on(o, "submittedQuantity")),
        "limitPrice": jo(on(o, "limitPrice")),
        "stopPrice": jo(on(o, "stopPrice")),
        "tif": f(o, "timeInForce").to_uppercase(),
        "currency": f(o, "securityCurrency").to_uppercase(),
        "accountId": s(or_v(o.get("canonicalAccountId"), o.get("accountId"))),
        "securityId": f(o, "securityId"),
        "type": f(o, "orderType").to_uppercase(),
    }))
}

pub(super) fn fetch_extended_order(sess: &Value, external_id: &str) -> Result<Option<Value>, CallError> {
    Ok(parse_extended_order(&gql(sess, "FetchSoOrdersExtendedOrder", json!({"branchId": ORDER_BRANCH, "externalId": external_id}))?))
}

pub(super) fn fetch_order_feed(sess: &Value, identity: &str) -> Result<Vec<Value>, CallError> {
    let mut out = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let data = gql(sess, "OrderServiceExtendedOrderFeed", json!({"identityId": identity, "statuses": WS_PENDING, "first": 25, "cursor": cursor}))?;
        let empty = json!({});
        let feed = data.get("identity").and_then(|v| v.get("orderServiceExtendedOrderFeed")).filter(|v| v.is_object()).unwrap_or(&empty);
        if let Some(edges) = feed.get("edges").and_then(|v| v.as_array()) {
            for edge in edges {
                if let Some(node) = edge.get("node") {
                    if node.is_object() && tr(node, "id") {
                        out.push(node.clone());
                    }
                }
            }
        }
        let page = feed.get("pageInfo").filter(|v| v.is_object()).unwrap_or(&empty);
        cursor = gv(page, "endCursor");
        if !tr(page, "hasNextPage") || !truthy(Some(&cursor)) {
            break;
        }
    }
    Ok(out)
}

pub(super) fn feed_order_row(node: &Value) -> Value {
    let empty = json!({});
    let sec = node.get("security").filter(|v| v.is_object()).unwrap_or(&empty);
    let stock = sec.get("stock").filter(|v| v.is_object()).unwrap_or(&empty);
    let acct = order_accounts(None).into_iter().find(|a| f(a, "id") == f(node, "canonicalAccountId"));
    let side = f(node, "side").to_uppercase();
    let sec_id = s(or_v(node.get("securityId"), sec.get("id")));
    let mut symbol = must(so::symbol_for_security(&db(), &sec_id));
    if symbol.is_empty() {
        symbol = s(or_v(node.get("symbol"), stock.get("symbol")));
    }
    let typ = f(node, "executionType").to_uppercase();
    json!({
        "id": f(node, "id"),
        "createdAt": f(node, "createdAtUtc"),
        "accountId": f(node, "canonicalAccountId"),
        "account": acct.map(|a| f(&a, "name")).unwrap_or_default(),
        "securityId": sec_id,
        "symbol": symbol,
        "currency": f(node, "securityCurrency").to_uppercase(),
        "side": if side.starts_with("SELL") { "SELL" } else { "BUY" },
        "type": if typ.is_empty() { "LIMIT".to_string() } else { typ },
        "quantity": num(node.get("submittedQuantity"), Some(0.0)).unwrap_or(0.0),
        "limitPrice": jo(on(node, "limitPrice")),
        "stopPrice": jo(on(node, "stopPrice")),
        "tif": "",
        "stopLoss": null,
        "takeProfit": null,
        "status": app_status(&f(node, "status")),
        "wsStatus": f(node, "status").to_uppercase(),
        "wsOrderId": f(node, "orderId"),
        "avgFill": jo(on(node, "averageFillPrice")),
        "source": "wealthsimple",
    })
}

pub static REFRESHED_AT: Mutex<String> = Mutex::new(String::new());
pub(super) static REFRESHING: AtomicBool = AtomicBool::new(false);

pub(super) fn refreshed_at() -> String {
    REFRESHED_AT.lock().unwrap().clone()
}

pub fn kick_orders_refresh() -> bool {
    let at = refreshed_at();
    if !at.is_empty() {
        let age = match parse_z(&at) {
            Some(t) => now_unix() - t as f64,
            None => ORDERS_REFRESH_SEC as f64,
        };
        if age < ORDERS_REFRESH_SEC as f64 {
            return false;
        }
    }
    if !connected_not_syncing() {
        return false;
    }
    if REFRESHING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return false;
    }
    spawn("bagholder-orders-refresh", || {
        let _ = catch_unwind(|| refresh_orders(""));
        REFRESHING.store(false, Ordering::SeqCst);
    });
    true
}

pub fn book_order_fill(order: &Value, upd: &Value) -> bool {
    if !order.is_object() {
        return false;
    }
    if f(order, "source") == "wealthsimple" {
        return false;
    }
    let side = f(order, "side").to_uppercase();
    if side != "BUY" && side != "SELL" {
        return false;
    }
    let account_id = f(order, "accountId");
    if !bagholder_store::activities::is_real_account(&account_id) {
        return false;
    }
    if f(order, "securityId").is_empty() {
        return false;
    }
    let symbol = s(or_v(upd.get("symbol"), order.get("symbol"))).trim().to_string();
    if symbol.is_empty() {
        return false;
    }
    let pick = |k: &str| if upd.get(k).map_or(false, |v| !v.is_null()) { upd.get(k) } else { order.get(k) };
    let filled = or_f(num(pick("filledQty"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    let price = or_f(num(pick("avgFill"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    if filled <= 0.0 || price <= 0.0 {
        return false;
    }
    let already = or_f(num(order.get("fillBookedQty"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    if already + 1e-9 >= filled {
        return false;
    }
    let fill_time = or_v(or_v(upd.get("lastFilledAt"), upd.get("firstFilledAt")), order.get("submittedAt"));
    let mut date = date_only(fill_time);
    if date.is_empty() {
        date = crate::app::today_utc();
    }
    let mut currency = upper(or_v(or_v(order.get("currency"), upd.get("currency")), Some(&json!("CAD"))));
    if currency != "CAD" && currency != "USD" {
        currency = "CAD".into();
    }
    let accounts = snapshot().get("accounts").cloned().unwrap_or(json!([]));
    let mult = bagholder_model::symbols::option_multiplier(&symbol);
    let buy = side == "BUY";
    let fifo = bagholder_ws::mapping::fifo_pool_ids(Some(&accounts)).get(&account_id).cloned().unwrap_or_else(|| account_id.clone());
    let act = json!({
        "id": uuid4(),
        "occurredAt": date,
        "transactionDate": date,
        "settlementDate": date,
        "accountId": account_id,
        "bookId": account_id,
        "fifoId": fifo,
        "accountType": bagholder_ws::mapping::account_type(&account_id, Some(&accounts)),
        "activityType": "Trade",
        "activitySubType": side,
        "description": format!("{} {} {} @ {}", if buy { "Buy" } else { "Sell" }, qty_text(filled), symbol, rp(Some(price))),
        "direction": if buy { "DEBIT" } else { "CREDIT" },
        "symbol": symbol,
        "name": symbol,
        "currency": currency,
        "quantity": if buy { filled } else { -filled },
        "unitPrice": price,
        "commission": 0.0,
        "netCashAmount": if buy { -(filled * price * mult) } else { filled * price * mult },
        "category": "trade",
        "balance": null,
        "securityId": f(order, "securityId"),
        "source": "bagholder-fill",
    });
    let conn = db();
    must(bagholder_store::activities::insert_local(&conn, &act, &uuid4));
    must(so::mark_order_fill_booked(&conn, &f(order, "id"), filled, &now_iso()));
    log(&format!("bagholder orders: {} filled {} {} @ {} booked as a local trade until the next sync", f(order, "id"), qty_text(filled), symbol, rp(Some(price))));
    true
}

pub fn refresh_orders(only_id: &str) -> Value {
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "skipped": "no session"}),
    };
    let live: Vec<Value> = list_orders().into_iter().filter(|o| is_live(o) && (only_id.is_empty() || f(o, "id") == only_id)).collect();
    let (mut read, mut failed, mut added) = (0i64, 0i64, 0i64);
    for o in &live {
        let oid = f(o, "id");
        let upd = match fetch_extended_order(&sess, &oid) {
            Ok(u) => u,
            Err(CallError::NotAuthorized) => {
                log("bagholder orders: Wealthsimple refused the session");
                return json!({"ok": false, "skipped": "refused"});
            }
            Err(e) => {
                failed += 1;
                log(&format!("bagholder orders: {} status failed: {}", oid, err_text(&e)));
                continue;
            }
        };
        let upd = match upd {
            Some(u) => u,
            None => continue,
        };
        let mut patch = Map::new();
        for k in ["wsStatus", "status", "filledQty", "avgFill", "submittedAt", "expiresAt"] {
            if upd.get(k).map_or(false, |v| !v.is_null()) {
                patch.insert(k.into(), upd[k].clone());
            }
        }
        if tr(&upd, "error") {
            patch.insert("error".into(), upd["error"].clone());
        }
        let role = f(o, "role");
        if f(o, "source") == "wealthsimple" || role == "stop" || role == "target" {
            for k in ["tif", "quantity", "limitPrice", "stopPrice", "currency"] {
                match upd.get(k) {
                    None | Some(Value::Null) => {}
                    Some(Value::String(t)) if t.is_empty() => {}
                    Some(v) => {
                        patch.insert(k.into(), v.clone());
                    }
                }
            }
            let name = must(so::symbol_for_security(&db(), &f(o, "securityId")));
            if !name.is_empty() && name != f(o, "symbol") {
                patch.insert("symbol".into(), json!(name));
            }
        }
        let notice = order_notice(o, &upd);
        update_order(&oid, Value::Object(patch));
        read += 1;
        if let Some((kind, key, title, body)) = notice {
            emit(&kind, &key, &title, &body);
        }
        if f(&upd, "status") == "filled" {
            let current = get_order(&oid).unwrap_or_else(|| o.clone());
            match catch_unwind(AssertUnwindSafe(|| book_order_fill(&current, &upd))) {
                Ok(_) => {}
                Err(_) => log(&format!("bagholder orders: {} fill not booked locally", oid)),
            }
        }
    }
    if only_id.is_empty() {
        let identity = identity_from(&sess);
        if !identity.is_empty() {
            let rows = list_orders();
            let mut known: HashSet<String> = rows.iter().map(|o| f(o, "id")).collect();
            known.extend(rows.iter().filter(|o| tr(o, "wsOrderId")).map(|o| f(o, "wsOrderId")));
            match fetch_order_feed(&sess, &identity) {
                Ok(nodes) => {
                    for node in nodes {
                        if known.contains(&f(&node, "id")) || known.contains(&f(&node, "orderId")) {
                            continue;
                        }
                        insert_order(&feed_order_row(&node));
                        added += 1;
                    }
                }
                Err(CallError::NotAuthorized) => return json!({"ok": false, "skipped": "refused"}),
                Err(e) => {
                    failed += 1;
                    log(&format!("bagholder orders: pending-order feed failed: {}", err_text(&e)));
                }
            }
        }
        *REFRESHED_AT.lock().unwrap() = now_iso();
    }
    if read != 0 || added != 0 || failed != 0 {
        log(&format!("bagholder orders: {} read, {} found pending at Wealthsimple, {} failed", read, added, failed));
    }
    json!({"ok": failed == 0, "read": read, "added": added, "failed": failed})
}

pub fn orders_loop() {
    while !app().wait(Duration::from_secs(ORDERS_REFRESH_SEC)) {
        if !connected_not_syncing() {
            continue;
        }
        let r = catch_unwind(|| {
            // Wealthsimple pushes nothing, so orders are read back. Closely while it
            // matters -- an order is live, or the panel is open on some page -- and
            // otherwise only often enough to hear of an order placed in Wealthsimple's
            // own app, which the fills notification is owed.
            let closely = list_orders().iter().any(is_live) || crate::events::watched("orders");
            let age = parse_z(&refreshed_at()).map(|t| now_unix() - t as f64);
            if !closely && age.map_or(false, |a| a < (ORDERS_REFRESH_SEC * 10) as f64) {
                return;
            }
            refresh_orders("");
        });
        if r.is_err() {
            log("bagholder orders: refresh failed");
        }
    }
}

pub fn cancel_order(order_id: &str) -> Value {
    #[cfg(test)]
    if let Some(v) = bracket_seam::CANCEL_ORDER.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return v;
    }
    let row = match get_order(order_id) {
        Some(r) => r,
        None => return json!({"ok": false, "error": "No such order."}),
    };
    if !is_live(&row) {
        return json!({"ok": false, "error": "That order is not open."});
    }
    if !orders_live() {
        return json!({"ok": false, "error": "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple."});
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "error": "Not connected."}),
    };
    let id = f(&row, "id");
    let data = match gql(&sess, "SoOrdersOrderCancel", json!({"cancelOrderRequest": {"externalId": id}})) {
        Ok(d) => d,
        Err(CallError::NotAuthorized) => return json!({"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}),
        Err(e) => {
            let msg = err_text(&e);
            log(&format!("bagholder orders: cancel {} failed: {}", id, msg));
            return json!({"ok": false, "error": format!("Cancel failed: {}", msg)});
        }
    };
    if let Some(msg) = data.get("orderServiceCancelOrder").and_then(|r| r.get("errors")).filter(|v| truthy(Some(v))).and_then(first_error) {
        log(&format!("bagholder orders: cancel {} refused: {}", id, msg));
        return json!({"ok": false, "error": format!("Wealthsimple refused the cancel: {}", msg)});
    }
    update_order(&id, json!({"status": "cancelling", "wsStatus": "CANCEL_PENDING"}));
    log(&format!("bagholder orders: cancel {} accepted", id));
    let rid = id.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(|| refresh_orders(&rid));
    });
    json!({"ok": true, "id": id, "status": "cancelling"})
}

pub fn orders_payload(kick: bool) -> Value {
    if kick {
        kick_orders_refresh();
    }
    let exchanges: HashMap<String, String> = must(bagholder_store::admin::list_securities(&db())).iter().map(|s| (f(s, "id"), f(s, "primaryExchange"))).collect();
    let mut orders = list_orders();
    for o in orders.iter_mut() {
        let ex = exchanges.get(&f(o, "securityId")).cloned().unwrap_or_default();
        set(o, "exchange", json!(ex));
    }
    json!({"ok": true, "orders": orders, "brackets": brackets(&[]), "live": orders_live(), "refreshedAt": refreshed_at()})
}

pub fn open_orders_count() -> i64 {
    let entries = list_orders().iter().filter(|o| is_live(o) && (o.get("role").is_none() || f(o, "role") == "entry")).count();
    let live = brackets(&[]).iter().filter(|b| BRACKET_LIVE.contains(&f(b, "status").as_str()) && f(b, "status") != "waiting").count();
    (entries + live) as i64
}
