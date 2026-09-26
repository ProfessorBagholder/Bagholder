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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

/// What Wealthsimple says of an order, or nothing when it names no status.
pub fn parse_extended_order(data: &wire::ExtendedOrderAnswer) -> Option<Reading> {
    let o = data.so_orders_extended_order.as_ref().filter(|o| !o.status.is_empty())?;
    let first = |a: &String, b: &String| if a.is_empty() { b.clone() } else { a.clone() };
    Some(Reading {
        ws_status: o.status.to_uppercase(),
        status: app_status(&o.status),
        filled_qty: o.filled_quantity,
        avg_fill: o.average_filled_price,
        submitted_at: o.submitted_at_utc.clone(),
        expires_at: o.expired_at_utc.clone(),
        first_filled_at: o.first_filled_at_utc.clone(),
        last_filled_at: o.last_filled_at_utc.clone(),
        error: first(&o.rejection_cause, &o.rejection_code),
        quantity: o.submitted_quantity,
        limit_price: o.limit_price,
        stop_price: o.stop_price,
        tif: o.time_in_force.to_uppercase(),
        currency: o.security_currency.to_uppercase(),
        account_id: first(&o.canonical_account_id, &o.account_id),
        security_id: o.security_id.clone(),
        kind: o.order_type.to_uppercase(),
    })
}

pub(super) fn fetch_extended_order(app: &Arc<App>, sess: &bagholder_ws::session::Session, external_id: &str) -> Result<Option<Reading>, CallError> {
    let data: wire::ExtendedOrderAnswer = gql_as(app, sess, "FetchSoOrdersExtendedOrder", json!({"branchId": ORDER_BRANCH, "externalId": external_id}))?;
    Ok(parse_extended_order(&data))
}

/// Every order the feed lists as still open, page by page.
pub(crate) fn fetch_order_feed(app: &Arc<App>, sess: &bagholder_ws::session::Session, identity: &str) -> Result<Vec<wire::FeedOrder>, CallError> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let data: wire::OrderFeedAnswer = gql_as(app, sess, "OrderServiceExtendedOrderFeed", json!({"identityId": identity, "statuses": WS_PENDING, "first": 25, "cursor": cursor}))?;
        let feed = data.identity.and_then(|i| i.order_service_extended_order_feed).unwrap_or_default();
        cursor = feed.next_cursor();
        out.extend(feed.nodes().filter(|n| !n.id.is_empty()));
        if cursor.is_none() {
            break;
        }
    }
    Ok(out)
}

/// An order found open at Wealthsimple that was not placed here. The feed is asked
/// only for open orders, so one that names no status is still open.
pub(crate) fn feed_order_row(app: &Arc<App>, node: &wire::FeedOrder) -> Order {
    let (sec_id, stock_symbol) = match &node.security {
        Some(sec) => (sec.id.clone(), sec.stock.as_ref().map(|st| st.symbol.clone()).unwrap_or_default()),
        None => (String::new(), String::new()),
    };
    let acct = match order_accounts(app) {
        Ok(a) => a.into_iter().find(|a| a.id == node.canonical_account_id),
        Err(e) => {
            log(&format!("bagholder orders: the accounts could not be read for an order found at Wealthsimple: {e}"));
            None
        }
    };
    let security_id = if node.security_id.is_empty() { sec_id } else { node.security_id.clone() };
    let mut symbol = book_symbol(app, &security_id);
    if symbol.is_empty() {
        symbol = if node.symbol.is_empty() { stock_symbol } else { node.symbol.clone() };
    }
    let kind = OrderType::parse(&node.execution_type.to_uppercase());
    let status = app_status(&node.status);
    Order {
        id: node.id.clone(),
        created_at: node.created_at_utc.clone(),
        account_id: node.canonical_account_id.clone(),
        account: acct.map(|a| a.name).unwrap_or_default(),
        security_id,
        symbol,
        currency: node.security_currency.to_uppercase(),
        side: if node.side.to_uppercase().starts_with("SELL") { Side::Sell } else { Side::Buy },
        kind: if kind.is_set() { kind } else { OrderType::Limit },
        quantity: Some(node.submitted_quantity.unwrap_or(0.0)),
        limit_price: node.limit_price,
        stop_price: node.stop_price,
        status: if status.is_set() { status } else { OrderStatus::Pending },
        ws_status: node.status.to_uppercase(),
        ws_order_id: node.order_id.clone(),
        avg_fill: node.average_fill_price,
        source: Source::Wealthsimple,
        ..Order::default()
    }
}

pub(super) fn refreshed_at(app: &App) -> String {
    app.orders.refreshed_at.lock().unwrap().clone()
}

pub fn kick_orders_refresh(app: &Arc<App>) -> bool {
    let at = refreshed_at(app);
    if !at.is_empty() {
        let age = match parse_z(&at) {
            Some(t) => now_unix() - t as f64,
            None => ORDERS_REFRESH_SEC as f64,
        };
        if age < ORDERS_REFRESH_SEC as f64 {
            return false;
        }
    }
    if !orders_can_run(app) {
        return false;
    }
    if app.orders.refreshing.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return false;
    }
    let a = app.clone();
    spawn("bagholder-orders-refresh", move || {
        let _ = catch_unwind(AssertUnwindSafe(|| refresh_orders(&a, "")));
        a.orders.refreshing.store(false, Ordering::SeqCst);
    });
    true
}

/// A fill read back of an order placed here: Wealthsimple's own row for it is
/// pulled at once (`docs/plans/stage-3c-switch.md`, §3: the pull runs when an order
/// is read back filled), so the trade reaches the book as the broker records it,
/// never as the app's own arithmetic. Only what has filled beyond what was
/// already asked for, and only once -- the quantity asked for is kept on the
/// order. Whether a pull was asked.
pub fn book_order_fill(app: &Arc<App>, order: &Order, upd: &Reading) -> bool {
    if order.source == Source::Wealthsimple || !order.side.is_set() {
        return false;
    }
    let filled = upd.filled_qty.or(order.filled_qty).unwrap_or(0.0);
    if filled <= 0.0 || order.fill_booked_qty.unwrap_or(0.0) + 1e-9 >= filled {
        return false;
    }
    let conn = db(app);
    must(so::mark_order_fill_booked(&conn, &order.id, filled, &now_iso()));
    app.pull_asked.store(true, Ordering::SeqCst);
    app.events.signal();
    log(&format!("bagholder orders: {} filled {}: Wealthsimple's row for it is pulled", order.id, qty_text(filled)));
    true
}

/// Wealthsimple's order ids of the fills of Bagholder's own orders that have been
/// read back filled, for the broker's reads to see into the book.
pub fn own_fills_booked(app: &Arc<App>) -> Vec<String> {
    orders_all(app)
        .into_iter()
        .filter(|o| o.source != Source::Wealthsimple && o.fill_booked_qty.is_some_and(|q| q > 0.0) && !o.ws_order_id.is_empty())
        .map(|o| o.ws_order_id)
        .collect()
}

/// `POST /api/orders/refresh`.
#[derive(Debug, Serialize, TS)]
pub struct RefreshOrdersAnswer {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub skipped: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub read: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub added: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub failed: Option<i64>,
}

impl RefreshOrdersAnswer {
    fn skipped(why: &str) -> RefreshOrdersAnswer {
        RefreshOrdersAnswer { ok: false, skipped: Some(why.into()), read: None, added: None, failed: None }
    }
}

/// Take what Wealthsimple says of an order into its row: where it stands and what
/// filled, and, for an order not written here or one the engine placed, its terms as
/// Wealthsimple has them. The person is told of a change, and a fill's own row is
/// pulled for the book.
fn apply_reading(app: &Arc<App>, o: &Order, upd: &Reading) {
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
        let name = book_symbol(app, &o.security_id);
        if !name.is_empty() && name != o.symbol {
            patch.symbol = Some(name);
        }
    }
    let notice = order_notice(o, upd);
    patch_order(app, &o.id, patch);
    if let Some((kind, key, title, body)) = notice {
        emit(app, &kind, &key, &title, &body);
    }
    if upd.status == OrderStatus::Filled {
        let current = order(app, &o.id).unwrap_or_else(|| o.clone());
        if catch_unwind(AssertUnwindSafe(|| book_order_fill(app, &current, upd))).is_err() {
            log(&format!("bagholder orders: {} fill not booked locally", o.id));
        }
    }
}

/// What an order left `sending` reads as when Wealthsimple has no record of it.
pub const NEVER_REACHED: &str = "Wealthsimple has no record of it: the app stopped while sending it.";

/// Rows left `sending` by a run that stopped before Wealthsimple answered: written and
/// not being sent now (`OrdersState::sending`). Each may or may not have reached
/// Wealthsimple, so each is read back by the external id it was sent with, and
/// Wealthsimple's answer decides it: an order it holds takes its status (an entry that
/// asked for a stop or a target gets the bracket its send never lived to make); an
/// order it has no record of never reached it, and fails with that reason. A row that
/// cannot be read now stays as it is, for the next pass. (rows read, reads failed)
pub(super) fn settle_unanswered(app: &Arc<App>, sess: &bagholder_ws::session::Session) -> (i64, i64) {
    let in_flight = app.orders.sending.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let left: Vec<Order> = orders_all(app).into_iter().filter(|o| o.status == OrderStatus::Sending && !in_flight.contains(&o.id)).collect();
    let (mut read, mut failed) = (0, 0);
    for o in &left {
        match fetch_extended_order(app, sess, &o.id) {
            Ok(Some(upd)) => {
                apply_reading(app, o, &upd);
                read += 1;
                log(&format!("bagholder orders: {} was being sent when the app stopped; Wealthsimple has it as {}", o.id, upd.ws_status));
                let wants_exits = o.role == Role::Entry && (o.stop_loss.is_some() || o.take_profit.is_some());
                let placed = matches!(upd.status, OrderStatus::Pending | OrderStatus::Cancelling | OrderStatus::Filled) || (upd.filled_qty.unwrap_or(0.0) > 0.0);
                if wants_exits && placed && must(so::typed::bracket_for_order(&db(app), &o.id)).is_none() {
                    if let Some(entry) = order(app, &o.id) {
                        let b = create_bracket(app, &entry);
                        log(&format!("bagholder bracket: {} for {}: made for the entry the app stopped while sending", b.id, b.symbol));
                    }
                }
            }
            Ok(None) => {
                let reading = Reading { status: OrderStatus::Failed, error: NEVER_REACHED.into(), ..Reading::default() };
                let notice = order_notice(o, &reading);
                patch_order(app, &o.id, OrderPatch { status: Some(OrderStatus::Failed), error: Some(NEVER_REACHED.into()), ..OrderPatch::default() });
                read += 1;
                log(&format!("bagholder orders: {} was being sent when the app stopped; Wealthsimple has no record of it", o.id));
                if let Some((kind, key, title, body)) = notice {
                    emit(app, &kind, &key, &title, &body);
                }
            }
            Err(e) => {
                failed += 1;
                log(&format!("bagholder orders: {} was being sent when the app stopped, and could not be read back: {}", o.id, err_text(&e)));
            }
        }
    }
    (read, failed)
}

/// Whether a row is left `sending` by a run that stopped: one this run must settle.
pub(super) fn unanswered(app: &App, o: &Order) -> bool {
    o.status == OrderStatus::Sending && !app.orders.sending.lock().unwrap_or_else(|e| e.into_inner()).contains(&o.id)
}

pub fn refresh_orders(app: &Arc<App>, only_id: &str) -> RefreshOrdersAnswer {
    let sess = match ticket_session(app) {
        Some(s) => s,
        None => return RefreshOrdersAnswer::skipped("no session"),
    };
    let live: Vec<Order> = orders_all(app).into_iter().filter(|o| o.status.is_live() && (only_id.is_empty() || o.id == only_id)).collect();
    let (mut read, mut failed, mut added) = (0i64, 0i64, 0i64);
    for o in &live {
        let upd = match fetch_extended_order(app, &sess, &o.id) {
            Ok(u) => u,
            Err(CallError::NotAuthorized) => {
                log("bagholder orders: Wealthsimple refused the session");
                return RefreshOrdersAnswer::skipped("refused");
            }
            Err(e) => {
                failed += 1;
                log(&format!("bagholder orders: {} status failed: {}", o.id, err_text(&e)));
                continue;
            }
        };
        let Some(upd) = upd else { continue };
        apply_reading(app, o, &upd);
        read += 1;
    }
    if only_id.is_empty() {
        let (r, f) = settle_unanswered(app, &sess);
        read += r;
        failed += f;
        let identity = sess.identity();
        if !identity.is_empty() {
            let rows = orders_all(app);
            let mut known: HashSet<String> = rows.iter().map(|o| o.id.clone()).collect();
            known.extend(rows.iter().filter(|o| !o.ws_order_id.is_empty()).map(|o| o.ws_order_id.clone()));
            match fetch_order_feed(app, &sess, &identity) {
                Ok(nodes) => {
                    for node in nodes {
                        if known.contains(&node.id) || known.contains(&node.order_id) {
                            continue;
                        }
                        must(so::typed::insert_order(&db(app), &feed_order_row(app, &node), &now_iso()));
                        added += 1;
                    }
                }
                Err(CallError::NotAuthorized) => return RefreshOrdersAnswer::skipped("refused"),
                Err(e) => {
                    failed += 1;
                    log(&format!("bagholder orders: pending-order feed failed: {}", err_text(&e)));
                }
            }
        }
        *app.orders.refreshed_at.lock().unwrap() = now_iso();
    }
    if read != 0 || added != 0 || failed != 0 {
        log(&format!("bagholder orders: {} read, {} found pending at Wealthsimple, {} failed", read, added, failed));
    }
    RefreshOrdersAnswer { ok: failed == 0, skipped: None, read: Some(read), added: Some(added), failed: Some(failed) }
}

pub fn orders_loop(app: &Arc<App>) {
    while !app.wait(Duration::from_secs(ORDERS_REFRESH_SEC)) {
        if !orders_can_run(app) {
            continue;
        }
        let r = catch_unwind(AssertUnwindSafe(|| {
            // Wealthsimple pushes nothing, so orders are read back. Closely while it
            // matters -- an order is live, or the panel is open on some page -- and
            // otherwise only often enough to hear of an order placed in Wealthsimple's
            // own app, which the fills notification is owed.
            let closely = orders_all(app).iter().any(|o| o.status.is_live() || unanswered(app, o)) || app.events.watched("orders");
            let age = parse_z(&refreshed_at(app)).map(|t| now_unix() - t as f64);
            if !closely && age.map_or(false, |a| a < (ORDERS_REFRESH_SEC * 10) as f64) {
                return;
            }
            refresh_orders(app, "");
        }));
        if r.is_err() {
            log("bagholder orders: refresh failed");
        }
    }
}

pub fn cancel_order(app: &Arc<App>, order_id: &str) -> OrderActionAnswer {
    #[cfg(test)]
    if let Some(v) = bracket_seam::CANCEL_ORDER.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return serde_json::from_value(v).unwrap_or_else(|_| OrderActionAnswer::err("bad seam value"));
    }
    let row = match order(app, order_id) {
        Some(r) => r,
        None => return OrderActionAnswer::err("No such order."),
    };
    if !row.status.is_live() {
        return OrderActionAnswer::err("That order is not open.");
    }
    if !orders_live() {
        return OrderActionAnswer::err(ORDERS_OFF);
    }
    let sess = match ticket_session(app) {
        Some(s) => s,
        None => return OrderActionAnswer::err("Not connected."),
    };
    let id = row.id.clone();
    let data: wire::CancelOrderAnswer = match gql_as(app, &sess, "SoOrdersOrderCancel", json!({"cancelOrderRequest": {"externalId": id}})) {
        Ok(d) => d,
        Err(CallError::NotAuthorized) => return OrderActionAnswer::err("Wealthsimple refused the session. Connect Wealthsimple again."),
        Err(e) => {
            let msg = err_text(&e);
            log(&format!("bagholder orders: cancel {} failed: {}", id, msg));
            return OrderActionAnswer::err(format!("Cancel failed: {}", msg));
        }
    };
    if let Some(reason) = data.order_service_cancel_order.and_then(|r| r.errors.0) {
        log(&format!("bagholder orders: cancel {} refused: {}", id, reason));
        return OrderActionAnswer::err(refused_words("Wealthsimple refused the cancel", &reason));
    }
    patch_order(app, &id, OrderPatch { status: Some(OrderStatus::Cancelling), ws_status: Some("CANCEL_PENDING".into()), ..OrderPatch::default() });
    log(&format!("bagholder orders: cancel {} accepted", id));
    let rid = id.clone();
    let a = app.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(AssertUnwindSafe(|| refresh_orders(&a, &rid)));
    });
    OrderActionAnswer::cancelling(id)
}

/// An order as the Orders panel shows it: the order, and the venue its listing trades on.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS, bagholder_diff_derive::Diff)]
#[diff(key = id)]
pub struct OrderCard {
    #[serde(flatten)]
    pub order: Order,
    pub exchange: String,
}

/// The `orders` document: what the Orders panel is sent, and sent again as it changes.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct OrdersDoc {
    pub ok: bool,
    pub orders: Vec<OrderCard>,
    pub brackets: Vec<Bracket>,
    /// Whether orders are sent at all (`BAGHOLDER_DRY_ORDERS` turns them off).
    pub live: bool,
    pub refreshed_at: String,
}

pub fn orders_doc(app: &Arc<App>, kick: bool) -> OrdersDoc {
    if kick {
        kick_orders_refresh(app);
    }
    let exchanges: HashMap<String, String> = match crate::market_context::securities(app) {
        Ok(v) => v.into_iter().map(|s| (s.id, s.primary_exchange)).collect(),
        Err(e) => {
            log(&format!("bagholder orders: the book's securities could not be read: {e}"));
            HashMap::new()
        }
    };
    let orders = orders_all(app).into_iter().map(|order| OrderCard { exchange: exchanges.get(&order.security_id).cloned().unwrap_or_default(), order }).collect();
    OrdersDoc { ok: true, orders, brackets: must(so::typed::list_brackets(&db(app), &[])), live: orders_live(), refreshed_at: refreshed_at(app) }
}

pub fn orders_payload(app: &Arc<App>, kick: bool) -> OrdersDoc {
    orders_doc(app, kick)
}

/// What the header's badge counts: the Orders panel's Pending cards (`SPEC.md` §4,
/// Orders), so the two agree -- every order still with the broker or being sent that
/// is not a bracket's own exit, and every bracket at work -- in the accounts in scope, named
/// by the broker's id for each (`None`: every account).
pub fn open_orders_count(app: &Arc<App>, accounts: Option<&HashSet<String>>) -> i64 {
    let within = |account: &str| accounts.map_or(true, |a| a.contains(account));
    let entries = orders_all(app).iter().filter(|o| (o.status.is_live() || o.status == OrderStatus::Sending) && !matches!(o.role, Role::Stop | Role::Target) && within(&o.account_id)).count();
    let at_work = must(so::typed::list_brackets(&db(app), &[])).iter().filter(|b| b.status.is_live() && b.status != BracketStatus::Waiting && within(&b.account_id)).count();
    (entries + at_work) as i64
}

/// What the book calls a Wealthsimple security now (a contract by its terms, as
/// the book names it); empty when the book has not met it or cannot say.
fn book_symbol(app: &Arc<App>, security_id: &str) -> String {
    use bagholder_core::instrument::{RefScheme, Reference};
    let read = || -> Result<String, String> {
        let Some(f) = app.figures.get() else { return Ok(String::new()) };
        let book = f.book()?;
        let r = Reference::new(RefScheme::BrokerSecurity(bagholder_core::Broker::named("wealthsimple")), security_id);
        let Some(i) = book.instrument_by_ref(&r).map_err(|e| e.to_string())? else { return Ok(String::new()) };
        Ok(book.names(i).map_err(|e| e.to_string())?.last().map(|n| n.symbol.clone()).unwrap_or_default())
    };
    match read() {
        Ok(s) => s,
        Err(e) => {
            log(&format!("bagholder orders: what the book calls {security_id} could not be read: {e}"));
            String::new()
        }
    }
}
