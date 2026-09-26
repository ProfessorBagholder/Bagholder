//! Reading orders back from Wealthsimple (`SPEC.md` §4, Orders, Refresh): every order
//! the broker may still act on, by its own id, and the feed of pending orders, which
//! also lists those placed in Wealthsimple's own app; what the person is told when an
//! order moves; a fill's own row pulled at once; and the person's cancel.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use bagholder_book::orders::StoredOrder;
use bagholder_book::Book;
use bagholder_core::order::{Asker, BrokerStatus, OrderFold, OrderKind, OrderRole, OrderState, Side};
use bagholder_core::Dec;
use jiff::Timestamp;
use serde::Serialize;
use serde_json::{json, Value};
use ts_rs::TS;

use super::gate::{self, Found, Held};
use super::{emit, gql_as, log, orders_can_run, orders_live, ticket_session, OrderActionAnswer, ORDERS_OFF};
use crate::app::App;

pub const ORDERS_REFRESH_SEC: u64 = 30;
/// With nothing open and no one watching, the feed alone is read this often.
pub const ORDERS_IDLE_SEC: i64 = 300;

/// The statuses the pending-order feed is asked for: every word of Wealthsimple's for
/// an order still working (`gate::WS_STATUSES`).
pub const FEED_STATUSES: [&str; 8] = ["NEW", "PENDING_SUBMISSION", "PENDING_REVIEW", "PENDING_FUND_TRANSFER", "SUBMITTED", "PLACED", "PARTIALLY_FILLED", "CONTINGENT"];

// ---------------------------------------------------------------------------
// words
// ---------------------------------------------------------------------------

/// A quantity as `SPEC.md` §3 writes one: no decimals for a whole number, two under a
/// whole, six under one; thousands separated.
pub fn qty_words(q: Dec) -> String {
    let a = q.abs();
    let places = if a.places() == 0 {
        0
    } else if a < Dec::ONE {
        6
    } else {
        2
    };
    signed_digits(q, places)
}

/// A price as `SPEC.md` §3 writes one: two decimals; under $1 a third only when it is
/// not zero; five under $0.01.
pub fn price_words(p: Dec) -> String {
    let a = p.abs();
    let cent = Dec::new(1, 2).expect("0.01");
    let places = if a.is_zero() {
        2
    } else if a < cent {
        5
    } else if a < Dec::ONE && a.round(3, bagholder_core::Rounding::HalfUp).places() == 3 {
        3
    } else {
        2
    };
    let t = p.round(places, bagholder_core::Rounding::HalfUp);
    fixed(t, places, false)
}

fn signed_digits(v: Dec, places: u32) -> String {
    let t = v.round(places, bagholder_core::Rounding::HalfUp);
    let s = fixed(t.abs(), places, true);
    if t.is_negative() {
        format!("−{s}")
    } else {
        s
    }
}

/// `v` with exactly `places` decimals, thousands separated when asked.
fn fixed(v: Dec, places: u32, thousands: bool) -> String {
    let text = v.abs().to_text();
    let (whole, frac) = text.split_once('.').unwrap_or((text.as_str(), ""));
    let mut frac = frac.to_string();
    while (frac.len() as u32) < places {
        frac.push('0');
    }
    let whole = if thousands {
        let b = whole.as_bytes();
        let mut out = String::new();
        for (i, c) in b.iter().enumerate() {
            if i > 0 && (b.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(*c as char);
        }
        out
    } else {
        whole.to_string()
    };
    let sign = if v.is_negative() { "-" } else { "" };
    if places == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{frac}")
    }
}

/// `Buy 5 at 1.75 limit`, `Sell 3 stop 1.60 · limit 1.55`, `Buy 2 at market`.
pub fn order_words(side: Side, quantity: Dec, kind: OrderKind, limit: Option<Dec>, stop: Option<Dec>) -> String {
    let side = if side == Side::Buy { "Buy" } else { "Sell" };
    let p = |v: Option<Dec>| v.map_or_else(|| "—".to_string(), price_words);
    let how = match kind {
        OrderKind::Market => "at market".to_string(),
        OrderKind::Stop => format!("stop {}", p(stop)),
        OrderKind::StopLimit => format!("stop {} · limit {}", p(stop), p(limit)),
        OrderKind::Limit => format!("at {} limit", p(limit)),
    };
    format!("{side} {} {how}", qty_words(quantity))
}

// ---------------------------------------------------------------------------
// what follows an order's move
// ---------------------------------------------------------------------------

/// What the person is told when an order moves: (kind, key, title, body), or nothing.
/// An exit of a bracket's is the bracket's business: only its fill or its refusal is told.
pub fn order_notice(o: &StoredOrder, before: &OrderFold, account: &str) -> Option<(String, String, String, String)> {
    use OrderState::*;
    let r = &o.request;
    let (was, now) = (before.state, o.fold.state);
    let role = r.bracket.as_ref().map_or(OrderRole::Entry, |(_, role)| *role);
    let tail = if account.is_empty() { String::new() } else { format!(" · {account}") };
    let qty = if o.fold.filled.is_positive() { o.fold.filled } else { r.quantity };
    let at = o.fold.average.map(|p| format!(" at {}", price_words(p))).unwrap_or_default();
    let sym = &r.symbol;
    let id = &r.id;
    let words = || order_words(r.side, r.quantity, r.kind, r.limit_price, r.stop_price);
    if now == Filled && was != Filled {
        let did = if r.side == Side::Sell { "Sold" } else { "Bought" };
        let head = match role {
            OrderRole::Stop | OrderRole::Market => "Stopped out",
            OrderRole::Target => "Target hit",
            OrderRole::Entry => "Order filled",
        };
        return Some(("fills".into(), format!("order:{id}:filled"), format!("{head} · {sym}"), format!("{did} {}{at}{tail}", qty_words(qty))));
    }
    if matches!(now, Rejected | Failed) && !matches!(was, Rejected | Failed) {
        let title = if now == Rejected { "Order rejected" } else { "Order not sent" };
        let body = match &o.fold.why {
            Some(why) if !why.is_empty() => format!("{} · {why}", words()),
            _ => format!("{}{tail}", words()),
        };
        return Some(("problems".into(), format!("order:{id}:{}", now.as_str()), format!("{title} · {sym}"), body));
    }
    if role != OrderRole::Entry {
        return None;
    }
    if now == Expired && was != Expired {
        return Some(("problems".into(), format!("order:{id}:expired"), format!("Order expired · {sym}"), format!("{}{tail}", words())));
    }
    // a cancel the person asked for is not news; one Wealthsimple made is
    if now == Cancelled && !matches!(was, Cancelled | Cancelling) {
        return Some(("problems".into(), format!("order:{id}:cancelled"), format!("Order cancelled · {sym}"), format!("{}{tail}", words())));
    }
    let filled = o.fold.filled;
    if now.in_flight() && filled > before.filled && filled < r.quantity {
        return Some(("fills".into(), format!("order:{id}:partial:{}", filled.to_text()), format!("Partly filled · {sym}"), format!("{} of {}{at}{tail}", qty_words(filled), qty_words(r.quantity))));
    }
    None
}

/// After an event moved an order or changed what is known of it: the person told, a
/// newly filled quantity's own Wealthsimple row pulled at once, and the page told.
pub(crate) fn followed(app: &Arc<App>, _book: &Book, before: &OrderFold, after: &StoredOrder) {
    if after.fold.filled > before.filled {
        // booked with the reading (`Book::book_fill`): the figures take it in now
        if let Some(f) = app.figures.get() {
            if let Err(e) = f.record_changed(Timestamp::now()) {
                log(&format!("bagholder orders: the fill of {} is booked and the figures could not take it in: {e}", after.request.id));
            }
        }
        app.pull_asked.store(true, Ordering::SeqCst);
        log(&format!("bagholder orders: {} filled {}: Wealthsimple's row for it is pulled", after.request.id, after.fold.filled.to_text()));
    }
    let account = super::account_name(app, &after.request.broker_account);
    if let Some((kind, key, title, body)) = order_notice(after, before, &account) {
        emit(app, &kind, &key, &title, &body);
    }
    app.events.signal();
}

/// Wealthsimple's order ids of Bagholder's own orders that have filled, for the
/// broker's reads to see whether the book holds each fill's row yet.
pub fn own_fills_booked(app: &Arc<App>) -> Result<Vec<String>, String> {
    let Some(f) = app.figures.get() else { return Ok(Vec::new()) };
    let book = f.book()?;
    Ok(book.orders_filled().map_err(|e| e.to_string())?.into_iter().filter_map(|o| o.fold.broker_id).collect())
}

// ---------------------------------------------------------------------------
// orders placed elsewhere
// ---------------------------------------------------------------------------

/// An order Wealthsimple reports that was not placed here: from the pending-order
/// feed, and, once it leaves the feed, as a read-back by its id says it ended.
#[derive(Clone, Debug, PartialEq)]
pub struct Elsewhere {
    /// The feed's id for it, which it is read back by.
    pub id: String,
    pub broker_id: Option<String>,
    pub account: String,
    pub security: String,
    pub symbol: String,
    pub currency: String,
    pub side: Side,
    pub kind: OrderKind,
    pub quantity: Dec,
    pub limit_price: Option<Dec>,
    pub stop_price: Option<Dec>,
    pub state: OrderState,
    pub filled: Dec,
    pub average: Option<Dec>,
    pub created_at: Timestamp,
    /// When it was read ended.
    pub ended_at: Option<Timestamp>,
}

fn text<'a>(o: &'a serde_json::Map<String, Value>, k: &str) -> Result<&'a str, String> {
    match o.get(k) {
        Some(Value::String(s)) if !s.is_empty() => Ok(s),
        other => Err(format!("{k} is {}, not text", other.map_or("missing".to_string(), |v| v.to_string()))),
    }
}

fn opt_text<'a>(o: &'a serde_json::Map<String, Value>, k: &str) -> Result<Option<&'a str>, String> {
    match o.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(v) => Err(format!("{k} is {v}, not text")),
    }
}

/// One node of `OrderServiceExtendedOrderFeed`, read strictly.
pub fn read_feed_node(node: &Value) -> Result<Elsewhere, String> {
    let o = node.as_object().ok_or_else(|| format!("a feed node that is {node}"))?;
    let id = text(o, "id")?.to_string();
    let word = text(o, "status")?.to_uppercase();
    let status = gate::WS_STATUSES.iter().find(|(w, _)| *w == word).map(|(_, s)| *s).ok_or_else(|| format!("{id}: a status this build does not know: {word:?}"))?;
    if status != BrokerStatus::Open {
        return Err(format!("{id}: the pending-order feed lists an order that is {word}"));
    }
    let side_word = text(o, "side")?.to_uppercase();
    let side = if side_word.starts_with("BUY") {
        Side::Buy
    } else if side_word.starts_with("SELL") {
        Side::Sell
    } else {
        return Err(format!("{id}: a side this build does not know: {side_word:?}"));
    };
    let kind = match text(o, "executionType")?.to_uppercase().as_str() {
        "MARKET" => OrderKind::Market,
        "LIMIT" => OrderKind::Limit,
        "STOP" => OrderKind::Stop,
        "STOP_LIMIT" => OrderKind::StopLimit,
        other => return Err(format!("{id}: an order type this build does not know: {other:?}")),
    };
    let quantity = gate::number(o.get("submittedQuantity"), "submittedQuantity")?.ok_or_else(|| format!("{id}: no submittedQuantity"))?;
    let sec = o.get("security").and_then(Value::as_object);
    let security = match opt_text(o, "securityId")? {
        Some(s) => s.to_string(),
        None => sec.map(|s| text(s, "id")).transpose()?.ok_or_else(|| format!("{id}: no security"))?.to_string(),
    };
    let stock_symbol = sec.and_then(|s| s.get("stock")).and_then(Value::as_object).map(|st| opt_text(st, "symbol")).transpose()?.flatten();
    let symbol = opt_text(o, "symbol")?.or(stock_symbol).unwrap_or_default().to_string();
    let created_at = text(o, "createdAtUtc")?.parse().map_err(|e: jiff::Error| format!("{id}: createdAtUtc: {e}"))?;
    Ok(Elsewhere {
        broker_id: opt_text(o, "orderId")?.map(String::from),
        account: text(o, "canonicalAccountId")?.to_string(),
        security,
        symbol,
        currency: text(o, "securityCurrency")?.to_uppercase(),
        side,
        kind,
        quantity,
        limit_price: gate::number(o.get("limitPrice"), "limitPrice")?,
        stop_price: gate::number(o.get("stopPrice"), "stopPrice")?,
        state: if word == "PARTIALLY_FILLED" { OrderState::PartlyFilled } else { OrderState::Pending },
        filled: Dec::ZERO,
        average: gate::number(o.get("averageFillPrice"), "averageFillPrice")?,
        created_at,
        ended_at: None,
        id,
    })
}

/// Every order the feed lists as pending, page by page, read strictly.
pub(crate) fn read_feed(app: &Arc<App>, sess: &bagholder_ws::session::Session) -> Result<Vec<Elsewhere>, String> {
    let identity = sess.identity();
    if identity.is_empty() {
        return Err("the session names no identity to read the pending orders of".into());
    }
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let data: Value = gql_as(app, sess, "OrderServiceExtendedOrderFeed", json!({"identityId": identity, "statuses": FEED_STATUSES, "first": 25, "cursor": cursor})).map_err(|e| super::err_text(&e))?;
        let feed = data.pointer("/identity/orderServiceExtendedOrderFeed").ok_or("an answer with no orderServiceExtendedOrderFeed")?;
        let edges = feed.get("edges").and_then(Value::as_array).ok_or("a feed with no edges")?;
        for e in edges {
            out.push(read_feed_node(e.get("node").ok_or("an edge with no node")?)?);
        }
        let page = feed.get("pageInfo").and_then(Value::as_object).ok_or("a feed with no pageInfo")?;
        let more = page.get("hasNextPage").and_then(Value::as_bool).ok_or("pageInfo with no hasNextPage")?;
        if !more {
            break;
        }
        let next = page.get("endCursor").and_then(Value::as_str).filter(|c| !c.is_empty()).ok_or("a next page with no cursor")?.to_string();
        if cursor.as_deref() == Some(next.as_str()) {
            return Err("the feed named the same page twice".into());
        }
        cursor = Some(next);
    }
    Ok(out)
}

/// Take the feed's pending orders in: the app's own left out (they are in the book),
/// each one no longer listed read back once by its id for how it ended.
fn take_feed(app: &Arc<App>, book: &Book, listed: Vec<Elsewhere>, now: Timestamp) -> Result<usize, String> {
    let mut own_brokers: HashSet<String> = HashSet::new();
    for o in book.orders_in_flight().map_err(|e| e.to_string())? {
        if let Some(b) = o.fold.broker_id {
            own_brokers.insert(b);
        }
    }
    let mut fresh = Vec::new();
    for e in listed {
        let own = book.order(&e.id).map_err(|e| e.to_string())?.is_some() || e.broker_id.as_ref().is_some_and(|b| own_brokers.contains(b));
        if !own {
            fresh.push(e);
        }
    }
    let before: Vec<Elsewhere> = app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut kept: Vec<Elsewhere> = Vec::new();
    let mut failed = 0;
    for mut old in before {
        if fresh.iter().any(|f| f.id == old.id) {
            continue;
        }
        if old.ended_at.is_some() {
            kept.push(old);
            continue;
        }
        // gone from the feed: it ended; how is Wealthsimple's to say
        match gate::broker(app).read(app, &old.id) {
            Ok(Found::Order(r)) => {
                let state = match r.status {
                    BrokerStatus::Filled => OrderState::Filled,
                    BrokerStatus::Cancelled => OrderState::Cancelled,
                    BrokerStatus::Expired => OrderState::Expired,
                    BrokerStatus::Rejected => OrderState::Rejected,
                    // still working, the feed's page not showing it yet: kept as it was
                    BrokerStatus::Open => {
                        kept.push(old);
                        continue;
                    }
                    BrokerStatus::NotFound => continue,
                };
                old.state = state;
                old.filled = r.filled;
                old.average = r.average.or(old.average);
                old.ended_at = Some(now);
                if state == OrderState::Filled {
                    let did = if old.side == Side::Sell { "Sold" } else { "Bought" };
                    let at = old.average.map(|p| format!(" at {}", price_words(p))).unwrap_or_default();
                    let account = super::account_name(app, &old.account);
                    let tail = if account.is_empty() { String::new() } else { format!(" · {account}") };
                    emit(app, "fills", &format!("order:{}:filled", old.id), &format!("Order filled · {}", old.symbol), &format!("{did} {}{at}{tail}", qty_words(if old.filled.is_positive() { old.filled } else { old.quantity })));
                }
                kept.push(old);
            }
            Ok(Found::None) => {}
            Err(e) => {
                failed += 1;
                log(&format!("bagholder orders: {} left the pending feed and could not be read back: {e}", old.id));
                kept.push(old);
            }
        }
    }
    kept.extend(fresh);
    kept.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    *app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()) = kept;
    Ok(failed)
}

// ---------------------------------------------------------------------------
// the read-back
// ---------------------------------------------------------------------------

/// `POST /api/orders/refresh`: what the read found.
#[derive(Debug, Default, Serialize, TS)]
pub struct RefreshOrdersAnswer {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub skipped: Option<String>,
    pub read: i64,
    pub failed: i64,
}

/// Read back every order the broker may still act on, however many, and the feed of
/// pending orders.
pub fn refresh_orders(app: &Arc<App>) -> RefreshOrdersAnswer {
    let skipped = |why: &str| RefreshOrdersAnswer { ok: false, skipped: Some(why.into()), ..RefreshOrdersAnswer::default() };
    let Some(f) = app.figures.get() else { return skipped("the book is not open") };
    let book = match f.book() {
        Ok(b) => b,
        Err(e) => return skipped(&format!("the book could not be opened: {e}")),
    };
    let Some(sess) = ticket_session(app) else { return skipped("no session") };
    let now = Timestamp::now();
    let (mut read, mut failed) = (0i64, 0i64);
    match book.orders_in_flight() {
        Ok(orders) => {
            for o in orders {
                match gate::read_back(app, &book, &o.request.id, now) {
                    Ok(_) => read += 1,
                    Err(e) => {
                        failed += 1;
                        log(&format!("bagholder orders: {} could not be read back: {e}", o.request.id));
                    }
                }
            }
        }
        Err(e) => {
            failed += 1;
            log(&format!("bagholder orders: the orders in flight could not be read from the book: {e}"));
        }
    }
    match read_feed(app, &sess).and_then(|listed| take_feed(app, &book, listed, now)) {
        Ok(n) => failed += n as i64,
        Err(e) => {
            failed += 1;
            log(&format!("bagholder orders: the pending-order feed could not be read: {e}"));
        }
    }
    super::learn_units(app, &book, &sess);
    *app.orders.refreshed_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(now);
    app.events.signal();
    if read != 0 || failed != 0 {
        log(&format!("bagholder orders: {read} read, {failed} failed"));
    }
    RefreshOrdersAnswer { ok: failed == 0, skipped: None, read, failed }
}

/// Whether anything is open that the broker may still move: an order of the app's in
/// flight, or one placed elsewhere still pending.
fn anything_open(app: &Arc<App>) -> bool {
    let own = app.figures.get().and_then(|f| f.book().ok()).map(|b| b.orders_in_flight().map(|o| o.iter().any(|o| o.fold.state != OrderState::Dry)).unwrap_or(true)).unwrap_or(false);
    own || app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|e| e.ended_at.is_none())
}

/// A read when the panel opens and the last is older than a tick.
pub fn kick_orders_refresh(app: &Arc<App>) {
    let fresh = app.orders.refreshed_at.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| Timestamp::now().duration_since(t).as_secs() < ORDERS_REFRESH_SEC as i64);
    if !fresh {
        super::ask_read(app);
    }
}

/// Wealthsimple pushes nothing, so orders are read back: at once after one is sent,
/// every thirty seconds while one is open or the panel is watched, and every five
/// minutes otherwise, to hear of an order placed in Wealthsimple's own app.
pub fn orders_loop(app: &Arc<App>) {
    while !app.stopping() {
        let asked = app.events.park_until_or(app, Duration::from_secs(ORDERS_REFRESH_SEC), || app.orders.read_asked.load(Ordering::SeqCst));
        if app.stopping() {
            return;
        }
        app.orders.read_asked.store(false, Ordering::SeqCst);
        if !orders_can_run(app) {
            continue;
        }
        let closely = asked || anything_open(app) || app.events.watched("orders");
        let age = app.orders.refreshed_at.lock().unwrap_or_else(|e| e.into_inner()).map(|t| Timestamp::now().duration_since(t).as_secs());
        if !closely && age.is_some_and(|a| a < ORDERS_IDLE_SEC) {
            continue;
        }
        let r = refresh_orders(app);
        if let Some(why) = r.skipped {
            log(&format!("bagholder orders: not read back: {why}"));
        }
    }
}

// ---------------------------------------------------------------------------
// the person's cancel
// ---------------------------------------------------------------------------

/// `POST /api/order/cancel`: the person cancels an order of the app's own. A bracket's
/// exit is the bracket's, cancelled with it.
pub fn cancel_order(app: &Arc<App>, order_id: &str) -> OrderActionAnswer {
    let Some(f) = app.figures.get() else { return OrderActionAnswer::err("The book is not open.") };
    let book = match f.book() {
        Ok(b) => b,
        Err(e) => return OrderActionAnswer::err(format!("The book could not be opened: {e}")),
    };
    let o = match book.order(order_id) {
        Ok(Some(o)) => o,
        Ok(None) => return cancel_elsewhere(app, order_id),
        Err(e) => return OrderActionAnswer::err(format!("The book could not be read: {e}")),
    };
    if o.request.bracket.as_ref().is_some_and(|(_, r)| *r != OrderRole::Entry) {
        return OrderActionAnswer::err("That order is a bracket's; cancel the bracket.");
    }
    if !matches!(o.fold.state, OrderState::Pending | OrderState::PartlyFilled) {
        return OrderActionAnswer::err(if o.fold.state == OrderState::Cancelling { "Its cancel is already sent." } else { "That order is not open." });
    }
    if !orders_live(app) {
        return OrderActionAnswer::err(ORDERS_OFF);
    }
    match gate::cancel(app, &book, order_id, &Asker::Person, Timestamp::now()) {
        Ok(Ok(fold)) if fold.state == OrderState::Cancelling => {
            super::ask_read(app);
            OrderActionAnswer::with_status(order_id, "cancelling")
        }
        Ok(Ok(fold)) => OrderActionAnswer::err(match fold.why {
            Some(why) if !why.is_empty() => format!("Wealthsimple refused the cancel: {why}"),
            _ => "Wealthsimple refused the cancel.".into(),
        }),
        Ok(Err(Held::Dry)) => OrderActionAnswer::err(ORDERS_OFF),
        Ok(Err(Held::NotNow(why))) => OrderActionAnswer::err(format!("Not now: {why}.")),
        Ok(Err(held)) => OrderActionAnswer::err(format!("Not sent: {held:?}.")),
        Err(e) => OrderActionAnswer::err(format!("The cancel could not be recorded: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }


    use bagholder_book::orders::{OrderRequest, StoredOrder};
    use bagholder_core::order::{OrderEvent, Reading, TimeInForce};

    fn at() -> Timestamp {
        "2026-09-15T14:00:00Z".parse().unwrap()
    }

    fn placed(id: &str, side: Side, kind: OrderKind, qty: &str, limit: Option<&str>, stop: Option<&str>, role: Option<OrderRole>) -> StoredOrder {
        let events = [OrderEvent::Written { dry: false }, OrderEvent::Accepted { broker_id: "ws-1".into() }];
        StoredOrder {
            request: OrderRequest {
                id: id.into(),
                broker: "wealthsimple".into(),
                broker_account: "acct-1".into(),
                broker_security: "sec-1".into(),
                symbol: "QNC".into(),
                currency: bagholder_core::Currency::CAD,
                side,
                kind,
                quantity: d(qty),
                limit_price: limit.map(d),
                stop_price: stop.map(d),
                time_in_force: TimeInForce::Day,
                bracket: role.map(|r| ("bracket-1".to_string(), r)),
                request: json!({}),
            },
            created_at: at(),
            fold: OrderFold::of(&events).unwrap(),
            stated_price: None,
            stated_quantity: None,
            expires_at: None,
            updated_at: at(),
            refused: None,
        }
    }

    /// The order after `e`, and the notice that move tells.
    fn moved(o: &StoredOrder, e: OrderEvent) -> Option<(String, String, String, String)> {
        let before = o.fold.clone();
        let mut after = o.clone();
        after.fold.apply(&e).unwrap();
        order_notice(&after, &before, "🚀 Trading")
    }

    fn read(status: BrokerStatus, filled: &str, avg: Option<&str>) -> OrderEvent {
        OrderEvent::Read(Reading::of(status, d(filled), avg.map(d)))
    }

    fn t4(a: &str, b: &str, c: &str, e: &str) -> Option<(String, String, String, String)> {
        Some((a.into(), b.into(), c.into(), e.into()))
    }

    #[test]
    fn a_fill_is_told_by_the_orders_role_once() {
        let o = placed("o1", Side::Buy, OrderKind::Limit, "5", Some("1.75"), None, None);
        assert_eq!(moved(&o, read(BrokerStatus::Filled, "5", Some("1.75"))), t4("fills", "order:o1:filled", "Order filled · QNC", "Bought 5 at 1.75 · 🚀 Trading"));
        let stop = placed("o2", Side::Sell, OrderKind::Stop, "5", None, Some("1.66"), Some(OrderRole::Stop));
        assert_eq!(moved(&stop, read(BrokerStatus::Filled, "5", Some("1.6374"))), t4("fills", "order:o2:filled", "Stopped out · QNC", "Sold 5 at 1.64 · 🚀 Trading"));
        let target = placed("o3", Side::Sell, OrderKind::Limit, "5", Some("1.93"), None, Some(OrderRole::Target));
        assert_eq!(moved(&target, read(BrokerStatus::Filled, "5", Some("1.93"))).unwrap().2, "Target hit · QNC");
        let mut filled = o.clone();
        filled.fold.apply(&read(BrokerStatus::Filled, "5", Some("1.75"))).unwrap();
        assert_eq!(moved(&filled, read(BrokerStatus::Filled, "5", Some("1.75"))), None, "read back filled again: nothing new");
        let big = placed("o1", Side::Buy, OrderKind::Limit, "100", Some("64.5"), None, None);
        assert_eq!(moved(&big, read(BrokerStatus::Open, "40", Some("64.5"))), t4("fills", "order:o1:partial:40", "Partly filled · QNC", "40 of 100 at 64.50 · 🚀 Trading"));
        let mut part = big.clone();
        part.fold.apply(&read(BrokerStatus::Open, "40", Some("64.5"))).unwrap();
        assert_eq!(moved(&part, read(BrokerStatus::Open, "40", Some("64.5"))), None, "the same partial fill again");
    }

    #[test]
    fn problems_are_told_but_not_the_persons_own_cancel_nor_an_exits_end() {
        let o = placed("o1", Side::Buy, OrderKind::Limit, "5", Some("1.75"), None, None);
        let mut sending = o.clone();
        sending.fold = OrderFold::of(&[OrderEvent::Written { dry: false }]).unwrap();
        assert_eq!(
            moved(&sending, OrderEvent::Refused { why: "Limit price has too many decimal places. Max allowed: 2".into(), code: None }),
            t4("problems", "order:o1:rejected", "Order rejected · QNC", "Buy 5 at 1.75 limit · Limit price has too many decimal places. Max allowed: 2")
        );
        assert_eq!(moved(&sending, OrderEvent::NotSent { why: "Not connected.".into() }).unwrap().2, "Order not sent · QNC");
        assert_eq!(moved(&o, read(BrokerStatus::Expired, "0", None)), t4("problems", "order:o1:expired", "Order expired · QNC", "Buy 5 at 1.75 limit · 🚀 Trading"));
        assert_eq!(moved(&o, read(BrokerStatus::Cancelled, "0", None)), t4("problems", "order:o1:cancelled", "Order cancelled · QNC", "Buy 5 at 1.75 limit · 🚀 Trading"));
        let mut cancelling = o.clone();
        cancelling.fold.apply(&OrderEvent::CancelAsked).unwrap();
        assert_eq!(moved(&cancelling, read(BrokerStatus::Cancelled, "0", None)), None, "a cancel asked for here is not told");
        let stop = placed("o2", Side::Sell, OrderKind::Stop, "5", None, Some("1.66"), Some(OrderRole::Stop));
        assert_eq!(moved(&stop, read(BrokerStatus::Expired, "0", None)), None, "an exit's expiry is the bracket's to place again");
        assert_eq!(moved(&stop, read(BrokerStatus::Cancelled, "0", None)), None);
        let rejected = OrderEvent::Read(Reading { why: Some("no shares".into()), ..Reading::of(BrokerStatus::Rejected, Dec::ZERO, None) });
        assert_eq!(moved(&stop, rejected).unwrap().3, "Sell 5 stop 1.66 · no shares");
    }

    #[test]
    fn prices_and_quantities_are_written_as_spec_3_writes_them() {
        assert_eq!(price_words(d("49.85")), "49.85");
        assert_eq!(price_words(d("165.4")), "165.40");
        assert_eq!(price_words(d("0.625")), "0.625");
        assert_eq!(price_words(d("0.54")), "0.54");
        assert_eq!(price_words(d("0.00123")), "0.00123");
        assert_eq!(price_words(d("0")), "0.00");
        assert_eq!(qty_words(d("233580")), "233,580");
        assert_eq!(qty_words(d("957.9")), "957.90");
        assert_eq!(qty_words(d("0.000123")), "0.000123");
        assert_eq!(qty_words(d("-5")), "−5");
    }

    #[test]
    fn an_order_is_said_as_the_ticket_says_it() {
        assert_eq!(order_words(Side::Buy, d("5"), OrderKind::Limit, Some(d("1.75")), None), "Buy 5 at 1.75 limit");
        assert_eq!(order_words(Side::Sell, d("3"), OrderKind::StopLimit, Some(d("1.55")), Some(d("1.6"))), "Sell 3 stop 1.60 · limit 1.55");
        assert_eq!(order_words(Side::Buy, d("2"), OrderKind::Market, None, None), "Buy 2 at market");
    }

    #[test]
    fn a_feed_node_is_read_strictly() {
        let good = json!({"id": "order-x", "orderId": "ws-1", "canonicalAccountId": "a", "createdAtUtc": "2026-09-10T14:00:00.123Z", "status": "SUBMITTED", "side": "BUY_QUANTITY", "executionType": "LIMIT", "submittedQuantity": 5, "limitPrice": "1.75", "securityCurrency": "usd", "securityId": "sec-1", "symbol": "QNC"});
        let e = read_feed_node(&good).unwrap();
        assert_eq!((e.side, e.kind, e.quantity, e.limit_price, e.currency.as_str()), (Side::Buy, OrderKind::Limit, d("5"), Some(d("1.75")), "USD"));
        for (field, bad) in [("status", json!("WHATEVER")), ("status", json!("FILLED")), ("side", json!("HOLD")), ("executionType", json!("ICEBERG")), ("submittedQuantity", json!("five")), ("createdAtUtc", json!("yesterday"))] {
            let mut n = good.clone();
            n[field] = bad.clone();
            assert!(read_feed_node(&n).is_err(), "{field} {bad} is refused");
        }
    }
}

/// The person cancels an order placed in Wealthsimple's own app, shown from the feed.
fn cancel_elsewhere(app: &Arc<App>, order_id: &str) -> OrderActionAnswer {
    let known = app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|e| e.id == order_id).map(|e| e.state);
    match known {
        None => return OrderActionAnswer::err("No such order."),
        Some(OrderState::Pending | OrderState::PartlyFilled) => {}
        Some(OrderState::Cancelling) => return OrderActionAnswer::err("Its cancel is already sent."),
        Some(_) => return OrderActionAnswer::err("That order is not open."),
    }
    match gate::cancel_elsewhere(app, order_id) {
        Err(_) => OrderActionAnswer::err(ORDERS_OFF),
        Ok(gate::Sent::Refused { why, .. }) | Ok(gate::Sent::NotSent { why }) => OrderActionAnswer::err(if why.is_empty() { "Wealthsimple refused the cancel.".to_string() } else { format!("Wealthsimple refused the cancel: {why}") }),
        Ok(_) => {
            if let Some(e) = app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|e| e.id == order_id) {
                e.state = OrderState::Cancelling;
            }
            log(&format!("bagholder orders: cancel of {order_id} (placed in Wealthsimple) sent"));
            super::ask_read(app);
            OrderActionAnswer::with_status(order_id, "cancelling")
        }
    }
}
