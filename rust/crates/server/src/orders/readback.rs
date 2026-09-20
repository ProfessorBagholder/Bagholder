//! Reading orders back from Wealthsimple: what became of each, the fills booked from
//! them, cancelling one, and what the Orders panel is sent.

use super::*;

// --- reading orders back ---

pub(super) const ORDER_BRANCH: &str = "TR";
pub const WS_PENDING: [&str; 8] = ["NEW", "PENDING_SUBMISSION", "PENDING_REVIEW", "PENDING_FUND_TRANSFER", "SUBMITTED", "PLACED", "PARTIALLY_FILLED", "CONTINGENT"];
pub(super) const WS_CANCELLING: [&str; 1] = ["CANCEL_PENDING"];
pub const ORDERS_REFRESH_SEC: u64 = 30;

pub(super) fn ws_status_map(s: &str) -> Option<OrderStatus> {
    Some(match s {
        "FILLED" | "POSTED" => OrderStatus::Filled,
        "CANCELLED" | "DELETED" => OrderStatus::Cancelled,
        "EXPIRED" => OrderStatus::Expired,
        "REJECTED" => OrderStatus::Rejected,
        _ => return None,
    })
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

pub(super) fn order_words(o: &Order) -> String {
    let side = if o.side == Side::Buy { "Buy" } else { "Sell" };
    let how = match o.kind {
        OrderType::Market => "at market".to_string(),
        OrderType::Stop => format!("stop {}", price_words(o.stop_price)),
        OrderType::StopLimit => format!("stop {} · limit {}", price_words(o.stop_price), price_words(o.limit_price)),
        _ => format!("at {} limit", price_words(o.limit_price)),
    };
    format!("{} {} {}", side, qty_words(o.quantity), how)
}

/// What Wealthsimple says of an order, read from its extended order.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Reading {
    pub ws_status: String,
    pub status: OrderStatus,
    pub filled_qty: Option<f64>,
    pub avg_fill: Option<f64>,
    pub submitted_at: String,
    pub expires_at: String,
    pub first_filled_at: String,
    pub last_filled_at: String,
    pub error: String,
    pub quantity: Option<f64>,
    pub limit_price: Option<f64>,
    pub stop_price: Option<f64>,
    pub tif: String,
    pub currency: String,
    pub account_id: String,
    pub security_id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// What the person is told when an order changes: (kind, key, title, body), or nothing.
/// A leg's cancel or expiry is the bracket engine's business and says nothing here.
pub fn order_notice(before: &Order, upd: &Reading) -> Option<(String, String, String, String)> {
    use OrderStatus::*;
    let (was, now) = (before.status, upd.status);
    let sym = if before.symbol.is_empty() { "?" } else { before.symbol.as_str() };
    let role = if before.role.is_set() { before.role } else { Role::Entry };
    let tail = if before.account.is_empty() { String::new() } else { format!(" · {}", before.account) };
    let oid = &before.id;
    let qty = or_f(or_f(upd.filled_qty, before.filled_qty), Some(before.quantity.unwrap_or(0.0)));
    let at = match or_f(upd.avg_fill, before.avg_fill) {
        Some(p) if p != 0.0 => format!(" at {}", price_words(Some(p))),
        _ => String::new(),
    };
    if now == Filled && was != Filled {
        let did = if before.side == Side::Sell { "Sold " } else { "Bought " };
        let head = match role {
            Role::Stop => "Stopped out · ",
            Role::Target => "Target hit · ",
            _ => "Order filled · ",
        };
        return Some(("fills".into(), format!("order:{}:filled", oid), format!("{}{}", head, sym), format!("{}{}{}{}", did, qty_words(qty), at, tail)));
    }
    if matches!(now, Rejected | Failed) && !matches!(was, Rejected | Failed) {
        let reason = if upd.error.is_empty() { &before.error } else { &upd.error };
        let title = format!("{}{}", if now == Rejected { "Order rejected · " } else { "Order not sent · " }, sym);
        let body = format!("{}{}", order_words(before), if reason.is_empty() { tail.clone() } else { format!(" · {}", reason) });
        return Some(("problems".into(), format!("order:{}:{}", oid, now), title, body));
    }
    if matches!(role, Role::Stop | Role::Target) {
        return None;
    }
    if now == Expired && was != Expired {
        return Some(("problems".into(), format!("order:{}:expired", oid), format!("Order expired · {}", sym), format!("{}{}", order_words(before), tail)));
    }
    if now == Cancelled && !matches!(was, Cancelled | Cancelling) {
        return Some(("problems".into(), format!("order:{}:cancelled", oid), format!("Order cancelled · {}", sym), format!("{}{}", order_words(before), tail)));
    }
    let filled = upd.filled_qty.unwrap_or(0.0);
    if now.is_live() && filled > before.filled_qty.unwrap_or(0.0) && filled < before.quantity.unwrap_or(0.0) {
        return Some((
            "fills".into(),
            format!("order:{}:partial:{}", oid, qty_words(Some(filled))),
            format!("Partly filled · {}", sym),
            format!("{} of {}{}{}", qty_words(Some(filled)), qty_words(before.quantity), at, tail),
        ));
    }
    None
}

/// Wealthsimple's word for where an order stands, in Bagholder's: one it does not know is
/// an order still open, which is the safe reading of it.
pub fn app_status(ws_status: &str) -> OrderStatus {
    let s = ws_status.to_uppercase();
    if WS_PENDING.contains(&s.as_str()) {
        return OrderStatus::Pending;
    }
    if WS_CANCELLING.contains(&s.as_str()) {
        return OrderStatus::Cancelling;
    }
    match ws_status_map(&s) {
        Some(x) => x,
        None => if s.is_empty() { OrderStatus::Unset } else { OrderStatus::Pending },
    }
}

pub fn parse_extended_order(data: &Value) -> Option<Reading> {
    let o = data.get("soOrdersExtendedOrder")?;
    if !o.is_object() || !tr(o, "status") {
        return None;
    }
    Some(Reading {
        ws_status: f(o, "status").to_uppercase(),
        status: app_status(&f(o, "status")),
        filled_qty: on(o, "filledQuantity"),
        avg_fill: on(o, "averageFilledPrice"),
        submitted_at: f(o, "submittedAtUtc"),
        expires_at: f(o, "expiredAtUtc"),
        first_filled_at: f(o, "firstFilledAtUtc"),
        last_filled_at: f(o, "lastFilledAtUtc"),
        error: s(or_v(o.get("rejectionCause"), o.get("rejectionCode"))),
        quantity: on(o, "submittedQuantity"),
        limit_price: on(o, "limitPrice"),
        stop_price: on(o, "stopPrice"),
        tif: f(o, "timeInForce").to_uppercase(),
        currency: f(o, "securityCurrency").to_uppercase(),
        account_id: s(or_v(o.get("canonicalAccountId"), o.get("accountId"))),
        security_id: f(o, "securityId"),
        kind: f(o, "orderType").to_uppercase(),
    })
}

pub(super) fn fetch_extended_order(sess: &Value, external_id: &str) -> Result<Option<Reading>, CallError> {
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

/// An order found pending at Wealthsimple that was not placed here.
pub(super) fn feed_order_row(node: &Value) -> Order {
    let empty = json!({});
    let sec = node.get("security").filter(|v| v.is_object()).unwrap_or(&empty);
    let stock = sec.get("stock").filter(|v| v.is_object()).unwrap_or(&empty);
    let acct = order_accounts(None).into_iter().find(|a| f(a, "id") == f(node, "canonicalAccountId"));
    let security_id = s(or_v(node.get("securityId"), sec.get("id")));
    let mut symbol = must(so::symbol_for_security(&db(), &security_id));
    if symbol.is_empty() {
        symbol = s(or_v(node.get("symbol"), stock.get("symbol")));
    }
    let kind = OrderType::parse(&f(node, "executionType").to_uppercase());
    Order {
        id: f(node, "id"),
        created_at: f(node, "createdAtUtc"),
        account_id: f(node, "canonicalAccountId"),
        account: acct.map(|a| f(&a, "name")).unwrap_or_default(),
        security_id,
        symbol,
        currency: f(node, "securityCurrency").to_uppercase(),
        side: if f(node, "side").to_uppercase().starts_with("SELL") { Side::Sell } else { Side::Buy },
        kind: if kind.is_set() { kind } else { OrderType::Limit },
        quantity: Some(on(node, "submittedQuantity").unwrap_or(0.0)),
        limit_price: on(node, "limitPrice"),
        stop_price: on(node, "stopPrice"),
        status: app_status(&f(node, "status")),
        ws_status: f(node, "status").to_uppercase(),
        ws_order_id: f(node, "orderId"),
        avg_fill: on(node, "averageFillPrice"),
        source: Source::Wealthsimple,
        ..Order::default()
    }
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

/// Write a fill as a trade in the book, until the next sync brings Wealthsimple's own row
/// for it. Only an order placed here, only what has filled beyond what was already
/// booked, and only once -- the booked quantity is kept on the order.
pub fn book_order_fill(order: &Order, upd: &Reading) -> bool {
    if order.source == Source::Wealthsimple || !order.side.is_set() {
        return false;
    }
    if !bagholder_store::activities::is_real_account(&order.account_id) || order.security_id.is_empty() {
        return false;
    }
    let symbol = order.symbol.trim().to_string();
    if symbol.is_empty() {
        return false;
    }
    let filled = upd.filled_qty.or(order.filled_qty).unwrap_or(0.0);
    let price = upd.avg_fill.or(order.avg_fill).unwrap_or(0.0);
    if filled <= 0.0 || price <= 0.0 {
        return false;
    }
    if order.fill_booked_qty.unwrap_or(0.0) + 1e-9 >= filled {
        return false;
    }
    let fill_time = [&upd.last_filled_at, &upd.first_filled_at, &order.submitted_at].into_iter().find(|t| !t.is_empty()).cloned().unwrap_or_default();
    let mut date = date_only(Some(&json!(fill_time)));
    if date.is_empty() {
        date = crate::app::today_utc();
    }
    let currency = [&order.currency, &upd.currency].into_iter().map(|c| c.trim().to_uppercase()).find(|c| !c.is_empty()).filter(|c| c == "CAD" || c == "USD").unwrap_or_else(|| "CAD".into());
    let accounts = snapshot().get("accounts").cloned().unwrap_or(json!([]));
    let mult = bagholder_model::symbols::option_multiplier(&symbol);
    let buy = order.side == Side::Buy;
    let account_id = &order.account_id;
    let fifo = bagholder_ws::mapping::fifo_pool_ids(Some(&accounts)).get(account_id).cloned().unwrap_or_else(|| account_id.clone());
    let act = json!({
        "id": uuid4(),
        "occurredAt": date,
        "transactionDate": date,
        "settlementDate": date,
        "accountId": account_id,
        "bookId": account_id,
        "fifoId": fifo,
        "accountType": bagholder_ws::mapping::account_type(account_id, Some(&accounts)),
        "activityType": "Trade",
        "activitySubType": order.side.as_str(),
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
        "securityId": order.security_id,
        "source": "bagholder-fill",
    });
    let conn = db();
    must(bagholder_store::activities::insert_local(&conn, &act, &uuid4));
    must(so::mark_order_fill_booked(&conn, &order.id, filled, &now_iso()));
    log(&format!("bagholder orders: {} filled {} {} @ {} booked as a local trade until the next sync", order.id, qty_text(filled), symbol, rp(Some(price))));
    true
}

pub fn refresh_orders(only_id: &str) -> Value {
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "skipped": "no session"}),
    };
    let live: Vec<Order> = orders_all().into_iter().filter(|o| o.status.is_live() && (only_id.is_empty() || o.id == only_id)).collect();
    let (mut read, mut failed, mut added) = (0i64, 0i64, 0i64);
    for o in &live {
        let upd = match fetch_extended_order(&sess, &o.id) {
            Ok(u) => u,
            Err(CallError::NotAuthorized) => {
                log("bagholder orders: Wealthsimple refused the session");
                return json!({"ok": false, "skipped": "refused"});
            }
            Err(e) => {
                failed += 1;
                log(&format!("bagholder orders: {} status failed: {}", o.id, err_text(&e)));
                continue;
            }
        };
        let Some(upd) = upd else { continue };
        let said = |t: &String| Some(t.clone()).filter(|t| !t.is_empty());
        let mut patch = OrderPatch {
            ws_status: Some(upd.ws_status.clone()),
            status: Some(upd.status),
            submitted_at: Some(upd.submitted_at.clone()),
            expires_at: Some(upd.expires_at.clone()),
            filled_qty: upd.filled_qty.map(Some),
            avg_fill: upd.avg_fill.map(Some),
            error: said(&upd.error),
            ..OrderPatch::default()
        };
        // an order not written here, or one the engine placed, is as Wealthsimple has it
        if o.source == Source::Wealthsimple || matches!(o.role, Role::Stop | Role::Target) {
            patch.tif = said(&upd.tif);
            patch.currency = said(&upd.currency);
            patch.quantity = upd.quantity.map(Some);
            patch.limit_price = upd.limit_price.map(Some);
            patch.stop_price = upd.stop_price.map(Some);
            let name = must(so::symbol_for_security(&db(), &o.security_id));
            if !name.is_empty() && name != o.symbol {
                patch.symbol = Some(name);
            }
        }
        let notice = order_notice(o, &upd);
        patch_order(&o.id, patch);
        read += 1;
        if let Some((kind, key, title, body)) = notice {
            emit(&kind, &key, &title, &body);
        }
        if upd.status == OrderStatus::Filled {
            let current = order(&o.id).unwrap_or_else(|| o.clone());
            if catch_unwind(AssertUnwindSafe(|| book_order_fill(&current, &upd))).is_err() {
                log(&format!("bagholder orders: {} fill not booked locally", o.id));
            }
        }
    }
    if only_id.is_empty() {
        let identity = identity_from(&sess);
        if !identity.is_empty() {
            let rows = orders_all();
            let mut known: HashSet<String> = rows.iter().map(|o| o.id.clone()).collect();
            known.extend(rows.iter().filter(|o| !o.ws_order_id.is_empty()).map(|o| o.ws_order_id.clone()));
            match fetch_order_feed(&sess, &identity) {
                Ok(nodes) => {
                    for node in nodes {
                        if known.contains(&f(&node, "id")) || known.contains(&f(&node, "orderId")) {
                            continue;
                        }
                        must(so::typed::insert_order(&db(), &feed_order_row(&node), &now_iso()));
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
            let closely = orders_all().iter().any(|o| o.status.is_live()) || crate::events::watched("orders");
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
    let row = match order(order_id) {
        Some(r) => r,
        None => return json!({"ok": false, "error": "No such order."}),
    };
    if !row.status.is_live() {
        return json!({"ok": false, "error": "That order is not open."});
    }
    if !orders_live() {
        return json!({"ok": false, "error": "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple."});
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "error": "Not connected."}),
    };
    let id = row.id.clone();
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
    patch_order(&id, OrderPatch { status: Some(OrderStatus::Cancelling), ws_status: Some("CANCEL_PENDING".into()), ..OrderPatch::default() });
    log(&format!("bagholder orders: cancel {} accepted", id));
    let rid = id.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(|| refresh_orders(&rid));
    });
    json!({"ok": true, "id": id, "status": "cancelling"})
}

/// An order as the Orders panel shows it: the order, and the venue its listing trades on.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OrderCard {
    #[serde(flatten)]
    pub order: Order,
    pub exchange: String,
}

/// The `orders` document: what the Orders panel is sent, and sent again as it changes.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrdersDoc {
    pub ok: bool,
    pub orders: Vec<OrderCard>,
    pub brackets: Vec<Bracket>,
    /// Whether orders are sent at all (`BAGHOLDER_DRY_ORDERS` turns them off).
    pub live: bool,
    pub refreshed_at: String,
}

pub fn orders_doc(kick: bool) -> OrdersDoc {
    if kick {
        kick_orders_refresh();
    }
    let exchanges: HashMap<String, String> = must(bagholder_store::admin::list_securities(&db())).iter().map(|s| (f(s, "id"), f(s, "primaryExchange"))).collect();
    let orders = orders_all().into_iter().map(|order| OrderCard { exchange: exchanges.get(&order.security_id).cloned().unwrap_or_default(), order }).collect();
    OrdersDoc { ok: true, orders, brackets: must(so::typed::list_brackets(&db(), &[])), live: orders_live(), refreshed_at: refreshed_at() }
}

pub fn orders_payload(kick: bool) -> Value {
    serde_json::to_value(orders_doc(kick)).unwrap_or(Value::Null)
}

/// What the header's badge counts: entries still with the broker, and brackets at work.
pub fn open_orders_count() -> i64 {
    let entries = orders_all().iter().filter(|o| o.status.is_live() && o.role == Role::Entry).count();
    let at_work = must(so::typed::list_brackets(&db(), &[])).iter().filter(|b| b.status.is_live() && b.status != BracketStatus::Waiting).count();
    (entries + at_work) as i64
}
