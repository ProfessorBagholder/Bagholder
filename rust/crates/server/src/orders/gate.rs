//! The one gate every order request leaves through (`docs/architecture.md` §11,
//! `docs/plans/stage-4-execution.md`, "One gate").
//!
//! An order is written to the book before anything is sent; the request goes out
//! once (`bagholder_ws::session::Client::mutate`, never re-sent by the transport);
//! what came back is recorded as the broker's answer, or as unconfirmed when it is
//! not one, and a read-back by the order's own id settles an unconfirmed order.
//! Nothing is sent again for an exit while one is in flight, and a bracket that
//! sends more than it ever could in a minute is stopped. With orders off, nothing
//! passes: that is checked here and nowhere else.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bagholder_book::orders::OrderRequest;
use bagholder_book::Book;
use bagholder_core::order::{Asker, BrokerStatus, OrderEvent, OrderFold, OrderRole, OrderState, Reading};
use bagholder_core::Dec;
use jiff::{SignedDuration, Timestamp};
use serde_json::{json, Value};

use crate::app::App;

/// What a request that places, cancels or changes an order came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Sent {
    /// The broker took it; for a new order, under its own id when it named one.
    Accepted { broker_id: Option<String> },
    /// The broker refused it: nothing was done.
    Refused { why: String, code: Option<String> },
    /// What became of it is not known: the order is read back to find out.
    Unclear { why: String },
    /// It never left the app (no session).
    NotSent { why: String },
}

/// What a read-back of an order found.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    Order(Reading),
    /// The broker has no order by that id.
    None,
}

/// A broker's order operations, as the gate uses them. Wealthsimple's is the one
/// the app runs; a test hands in a fake that misbehaves.
pub trait OrderBroker: Send + Sync {
    fn create(&self, app: &Arc<App>, request: &Value) -> Sent;
    fn cancel(&self, app: &Arc<App>, external_id: &str) -> Sent;
    fn modify(&self, app: &Arc<App>, external_id: &str, change: &Value) -> Sent;
    /// Where the order stands, by the app's own id; `Err` when the broker could not be asked.
    fn read(&self, app: &Arc<App>, external_id: &str) -> Result<Found, String>;
}

/// At most this many requests a minute from one bracket; one more can only be a fault.
pub const BRACKET_CAP_PER_MINUTE: usize = 10;

/// Why the gate let nothing through.
#[derive(Clone, Debug, PartialEq)]
pub enum Held {
    /// Orders are off: the order is written as dry, nothing sent.
    Dry,
    /// An exit of the bracket is in flight: nothing more is placed for it.
    InFlight,
    /// The bracket reached its cap this minute.
    Capped,
    /// The order's state does not allow it (cancelling an order that is not working).
    NotNow(String),
}

/// The broker the app sends to: Wealthsimple, or a test's.
pub fn broker(app: &App) -> Arc<dyn OrderBroker> {
    app.orders.gate.broker.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| Arc::new(Wealthsimple))
}

/// The lock a bracket's decisions and sends are made under, one bracket at a time.
pub fn bracket_lock(app: &App, bracket: &str) -> Arc<Mutex<()>> {
    app.orders.gate.bracket_locks.lock().unwrap_or_else(|e| e.into_inner()).entry(bracket.to_string()).or_default().clone()
}

/// Requests a bracket sent in the minute before `now`: its orders written, and the
/// cancels asked for them.
fn sent_this_minute(book: &Book, bracket: &str, now: Timestamp) -> Result<usize, String> {
    let since = now - SignedDuration::from_secs(60);
    let mut n = 0;
    for o in book.orders_of_bracket(bracket).map_err(|e| e.to_string())? {
        if o.request.bracket.as_ref().is_some_and(|(_, r)| *r == OrderRole::Entry) {
            continue;
        }
        if o.created_at > since {
            n += 1;
        }
        n += book.order_log(&o.request.id).map_err(|e| e.to_string())?.iter().filter(|l| l.at > since && l.refused.is_none() && matches!(l.event, OrderEvent::CancelAsked)).count();
    }
    Ok(n)
}

/// Place an order: written, then sent once, its answer recorded. `Ok` with the
/// order's state after the answer; `Err(Held)` when nothing was written or sent.
pub fn place(app: &Arc<App>, book: &Book, o: &OrderRequest, asker: &Asker, now: Timestamp) -> Result<Result<OrderFold, Held>, String> {
    if let Some((bracket, role)) = &o.bracket {
        if *role != OrderRole::Entry {
            if book.orders_of_bracket(bracket).map_err(|e| e.to_string())?.iter().any(|x| x.request.bracket.as_ref().is_some_and(|(_, r)| *r != OrderRole::Entry) && x.fold.state.in_flight()) {
                return Ok(Err(Held::InFlight));
            }
            if sent_this_minute(book, bracket, now)? >= BRACKET_CAP_PER_MINUTE {
                return Ok(Err(Held::Capped));
            }
        }
    }
    let live = super::orders_live(app);
    book.write_order(o, !live, asker, now).map_err(|e| e.to_string())?;
    if !live {
        return Ok(Err(Held::Dry));
    }
    // until its answer is recorded, a read-back must not decide it: the broker may
    // not have it yet, and "no such order" would fail an order it is about to take
    let _sending = Sending::start(app, &o.id);
    let event = match broker(app).create(app, &o.request) {
        Sent::Accepted { broker_id: Some(id) } => OrderEvent::Accepted { broker_id: id },
        // taken, and no id named: the read-back finds it
        Sent::Accepted { broker_id: None } => OrderEvent::Unclear { why: "the broker took it and named no id".into() },
        Sent::Refused { why, code } => OrderEvent::Refused { why, code },
        Sent::Unclear { why } => OrderEvent::Unclear { why },
        Sent::NotSent { why } => OrderEvent::NotSent { why },
    };
    record(app, book, &o.id, asker, now, &event)?;
    Ok(Ok(fold(book, &o.id)?))
}

/// Ask for an order's cancel. `Ok` with its state after the answer.
pub fn cancel(app: &Arc<App>, book: &Book, id: &str, asker: &Asker, now: Timestamp) -> Result<Result<OrderFold, Held>, String> {
    if !super::orders_live(app) {
        return Ok(Err(Held::Dry));
    }
    let before = fold(book, id)?;
    if !matches!(before.state, OrderState::Pending | OrderState::PartlyFilled) {
        return Ok(Err(Held::NotNow(format!("the order is {}", before.state.as_str()))));
    }
    if let Some((bracket, role)) = book.order(id).map_err(|e| e.to_string())?.and_then(|o| o.request.bracket) {
        if role != OrderRole::Entry && sent_this_minute(book, &bracket, now)? >= BRACKET_CAP_PER_MINUTE {
            return Ok(Err(Held::Capped));
        }
    }
    record(app, book, id, asker, now, &OrderEvent::CancelAsked)?;
    match broker(app).cancel(app, id) {
        // asked, and taken: the read-back says when it is gone
        Sent::Accepted { .. } => {}
        // no answer: still being cancelled, as far as anyone knows; the read-back settles it
        Sent::Unclear { .. } => {}
        Sent::Refused { why, .. } | Sent::NotSent { why } => {
            record(app, book, id, asker, now, &OrderEvent::CancelRefused { why })?;
        }
    }
    Ok(Ok(fold(book, id)?))
}

/// Ask the broker where an order stands, and record what it says.
pub fn read_back(app: &Arc<App>, book: &Book, id: &str, now: Timestamp) -> Result<OrderFold, String> {
    let before = fold(book, id)?;
    if before.state == OrderState::Dry {
        return Ok(before);
    }
    // being sent by this run: its own answer settles it, not a read
    if app.orders.gate.sending.lock().unwrap_or_else(|e| e.into_inner()).contains(id) {
        return Ok(before);
    }
    let found = broker(app).read(app, id)?;
    let reading = match found {
        Found::Order(r) => r,
        Found::None => Reading::of(BrokerStatus::NotFound, Dec::ZERO, None),
    };
    record(app, book, id, &Asker::Engine, now, &OrderEvent::Read(reading))?;
    fold(book, id)
}

/// Record an event on an order, and what follows from it: the person told of a fill,
/// a refusal or an order ended at the broker, and a fill's own row pulled.
fn record(app: &Arc<App>, book: &Book, id: &str, asker: &Asker, now: Timestamp, e: &OrderEvent) -> Result<(), String> {
    let before = fold(book, id)?;
    match book.order_event(id, asker, now, e).map_err(|e| e.to_string())? {
        Err(refused) => crate::app::log(&format!("bagholder order {id}: {refused}")),
        Ok(_) => {
            let after = book.order(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no order {id}"))?;
            super::readback::followed(app, book, &before, &after);
        }
    }
    Ok(())
}

/// Change a working order's limit or quantity: asked, then sent once. `Ok` with its
/// state after the answer; what it stands at after is the broker's to say (the next read).
pub fn modify(app: &Arc<App>, book: &Book, id: &str, limit_price: Option<Dec>, quantity: Option<Dec>, asker: &Asker, now: Timestamp) -> Result<Result<OrderFold, Held>, String> {
    if !super::orders_live(app) {
        return Ok(Err(Held::Dry));
    }
    let before = fold(book, id)?;
    if !matches!(before.state, OrderState::Pending | OrderState::PartlyFilled) {
        return Ok(Err(Held::NotNow(format!("the order is {}", before.state.as_str()))));
    }
    record(app, book, id, asker, now, &OrderEvent::ModifyAsked { limit_price, quantity })?;
    let mut change = serde_json::Map::new();
    if let Some(p) = limit_price {
        change.insert("newLimitPrice".into(), json!(p.to_f64()));
    }
    if let Some(q) = quantity {
        change.insert("newQuantity".into(), json!(q.to_f64()));
    }
    match broker(app).modify(app, id, &Value::Object(change)) {
        Sent::Accepted { .. } | Sent::Unclear { .. } => {}
        Sent::Refused { why, .. } | Sent::NotSent { why } => record(app, book, id, asker, now, &OrderEvent::ModifyRefused { why })?,
    }
    Ok(Ok(fold(book, id)?))
}

fn fold(book: &Book, id: &str) -> Result<OrderFold, String> {
    Ok(book.order(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no order {id}"))?.fold)
}

// ---------------------------------------------------------------------------
// Wealthsimple
// ---------------------------------------------------------------------------

/// Wealthsimple's order operations, as its web app sends them.
pub struct Wealthsimple;

const ORDER_BRANCH: &str = "TR";

impl OrderBroker for Wealthsimple {
    fn create(&self, app: &Arc<App>, request: &Value) -> Sent {
        let Some(sess) = super::ticket_session(app) else { return Sent::NotSent { why: "Not connected.".into() } };
        created(mutate::<bagholder_ws::wire::CreateOrderAnswer>(app, &sess, "SoOrdersOrderCreate", &json!({ "input": request })))
    }

    fn cancel(&self, app: &Arc<App>, external_id: &str) -> Sent {
        let Some(sess) = super::ticket_session(app) else { return Sent::NotSent { why: "Not connected.".into() } };
        cancelled(mutate::<bagholder_ws::wire::CancelOrderAnswer>(app, &sess, "SoOrdersOrderCancel", &json!({ "cancelOrderRequest": { "externalId": external_id } })))
    }

    fn modify(&self, app: &Arc<App>, external_id: &str, change: &Value) -> Sent {
        let Some(sess) = super::ticket_session(app) else { return Sent::NotSent { why: "Not connected.".into() } };
        let mut input = change.clone();
        input["externalId"] = json!(external_id);
        modified(mutate::<bagholder_ws::wire::ModifyOrderAnswer>(app, &sess, "SoOrdersOrderModify", &json!({ "input": input })))
    }

    fn read(&self, app: &Arc<App>, external_id: &str) -> Result<Found, String> {
        let Some(sess) = super::ticket_session(app) else { return Err("Not connected.".into()) };
        let data: Value = super::gql_as(app, &sess, "FetchSoOrdersExtendedOrder", json!({"branchId": ORDER_BRANCH, "externalId": external_id})).map_err(|e| e.to_string())?;
        read_extended(&data)
    }
}


use bagholder_ws::session::Mutation;

fn refused_or(m: Mutation<()>) -> Sent {
    match m {
        Mutation::Answer(()) => Sent::Accepted { broker_id: None },
        Mutation::Refused(why) => Sent::Refused { why, code: None },
        Mutation::NotAuthorized => Sent::Refused { why: "The Wealthsimple session lapsed.".into(), code: None },
        Mutation::Unclear(why) => Sent::Unclear { why },
    }
}

fn split<T>(m: Mutation<T>) -> Result<T, Sent> {
    match m {
        Mutation::Answer(a) => Ok(a),
        Mutation::Refused(why) => Err(refused_or(Mutation::Refused(why))),
        Mutation::NotAuthorized => Err(refused_or(Mutation::NotAuthorized)),
        Mutation::Unclear(why) => Err(refused_or(Mutation::Unclear(why))),
    }
}

fn errors_of(e: &bagholder_ws::wire::Refusal) -> Option<Sent> {
    (e.0.is_some() || e.1.is_some()).then(|| Sent::Refused { why: e.0.clone().or(e.1.clone()).unwrap_or_default(), code: e.1.clone() })
}

/// What Wealthsimple's answer to a create says of the order.
pub fn created(m: Mutation<bagholder_ws::wire::CreateOrderAnswer>) -> Sent {
    match split(m) {
        Err(sent) => sent,
        Ok(a) => match a.so_orders_create_order {
            Some(r) => errors_of(&r.errors).unwrap_or(Sent::Accepted { broker_id: r.order.map(|o| o.order_id).filter(|id| !id.is_empty()) }),
            None => Sent::Unclear { why: "SoOrdersOrderCreate: an answer that names no order".into() },
        },
    }
}

/// What Wealthsimple's answer to a cancel says.
pub fn cancelled(m: Mutation<bagholder_ws::wire::CancelOrderAnswer>) -> Sent {
    match split(m) {
        Err(sent) => sent,
        Ok(a) => match a.order_service_cancel_order {
            Some(r) => errors_of(&r.errors).unwrap_or(Sent::Accepted { broker_id: None }),
            None => Sent::Unclear { why: "SoOrdersOrderCancel: an answer with nothing in it".into() },
        },
    }
}

/// What Wealthsimple's answer to a change says.
pub fn modified(m: Mutation<bagholder_ws::wire::ModifyOrderAnswer>) -> Sent {
    match split(m) {
        Err(sent) => sent,
        Ok(a) => match a.so_orders_modify_order {
            Some(r) => errors_of(&r.errors).unwrap_or(Sent::Accepted { broker_id: None }),
            None => Sent::Unclear { why: "SoOrdersOrderModify: an answer with nothing in it".into() },
        },
    }
}

#[cfg(not(test))]
fn mutate<T: serde::de::DeserializeOwned>(app: &Arc<App>, sess: &bagholder_ws::session::Session, op: &str, vars: &Value) -> bagholder_ws::session::Mutation<T> {
    let home = app.ws_home();
    bagholder_ws::session::Client { home: &home }.mutate(sess, op, vars)
}

/// Under test nothing reaches the network: a test hands the gate a fake broker.
#[cfg(test)]
fn mutate<T: serde::de::DeserializeOwned>(_app: &Arc<App>, _sess: &bagholder_ws::session::Session, op: &str, _vars: &Value) -> bagholder_ws::session::Mutation<T> {
    bagholder_ws::session::Mutation::Unclear(format!("{op}: no network in tests"))
}

/// Wealthsimple's words for where an order stands, and what each means here. A word
/// not listed is an answer this build cannot read: a failure, never a guess.
pub const WS_STATUSES: [(&str, BrokerStatus); 15] = [
    ("NEW", BrokerStatus::Open),
    ("PENDING_SUBMISSION", BrokerStatus::Open),
    ("PENDING_REVIEW", BrokerStatus::Open),
    ("PENDING_FUND_TRANSFER", BrokerStatus::Open),
    ("SUBMITTED", BrokerStatus::Open),
    ("PLACED", BrokerStatus::Open),
    ("PARTIALLY_FILLED", BrokerStatus::Open),
    ("CONTINGENT", BrokerStatus::Open),
    // a cancel asked and not yet done: the order still works until it is
    ("CANCEL_PENDING", BrokerStatus::Open),
    ("FILLED", BrokerStatus::Filled),
    ("POSTED", BrokerStatus::Filled),
    ("CANCELLED", BrokerStatus::Cancelled),
    ("DELETED", BrokerStatus::Cancelled),
    ("EXPIRED", BrokerStatus::Expired),
    ("REJECTED", BrokerStatus::Rejected),
];

/// A number as Wealthsimple states it: a JSON number or its text, read exactly.
pub(crate) fn number(v: Option<&Value>, field: &str) -> Result<Option<Dec>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Dec::parse(&n.to_string()).map(Some).map_err(|e| format!("{field}: {e}")),
        Some(Value::String(s)) if !s.trim().is_empty() => Dec::parse(s.trim()).map(Some).map_err(|e| format!("{field}: {e}")),
        Some(other) => Err(format!("{field} is {other}, not a number")),
    }
}

fn instant(v: Option<&Value>, field: &str) -> Result<Option<Timestamp>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => s.parse().map(Some).map_err(|e: jiff::Error| format!("{field} {s:?}: {e}")),
        Some(other) => Err(format!("{field} is {other}, not a time")),
    }
}

fn text_of(v: Option<&Value>, field: &str) -> Result<Option<String>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(format!("{field} is {other}, not text")),
    }
}

/// `FetchSoOrdersExtendedOrder`'s answer, read strictly.
pub fn read_extended(data: &Value) -> Result<Found, String> {
    let o = match data.get("soOrdersExtendedOrder") {
        None => return Err("an answer with no soOrdersExtendedOrder".into()),
        Some(Value::Null) => return Ok(Found::None),
        Some(Value::Object(o)) => o,
        Some(other) => return Err(format!("soOrdersExtendedOrder is {other}")),
    };
    let word = o.get("status").and_then(Value::as_str).ok_or("an order with no status")?.to_uppercase();
    let status = WS_STATUSES.iter().find(|(w, _)| *w == word).map(|(_, s)| *s).ok_or_else(|| format!("a status this build does not know: {word:?}"))?;
    let filled = number(o.get("filledQuantity"), "filledQuantity")?.unwrap_or(Dec::ZERO);
    // `orderType` states the side (`buy_quantity`), not how it is priced: a stop's
    // price is its stop, which only a stop or stop-limit order states
    let price = match number(o.get("stopPrice"), "stopPrice")? {
        Some(stop) => Some(stop),
        None => number(o.get("limitPrice"), "limitPrice")?,
    };
    // an order that expires reports when; a working good-till-cancelled one reports when it will
    Ok(Found::Order(Reading {
        status,
        filled,
        average: number(o.get("averageFilledPrice"), "averageFilledPrice")?,
        price,
        quantity: number(o.get("submittedQuantity"), "submittedQuantity")?,
        expires_at: instant(o.get("expiredAtUtc"), "expiredAtUtc")?,
        why: text_of(o.get("rejectionCause"), "rejectionCause")?,
        code: text_of(o.get("rejectionCode"), "rejectionCode")?,
    }))
}

/// The per-bracket locks and the broker the gate sends to (a test's fake, or
/// Wealthsimple when none is set).
#[derive(Default)]
pub struct GateState {
    pub broker: Mutex<Option<Arc<dyn OrderBroker>>>,
    pub bracket_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// The orders this run is sending now, whose create has not been answered yet.
    pub sending: Mutex<HashSet<String>>,
}

/// An order marked as being sent for as long as this lives.
struct Sending<'a> {
    app: &'a App,
    id: String,
}

impl<'a> Sending<'a> {
    fn start(app: &'a App, id: &str) -> Sending<'a> {
        app.orders.gate.sending.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string());
        Sending { app, id: id.to_string() }
    }
}

impl Drop for Sending<'_> {
    fn drop(&mut self) {
        self.app.orders.gate.sending.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

/// Cancel an order placed elsewhere (in Wealthsimple's own app): not the app's, so
/// not in the book, but sent once all the same, and only from here.
pub fn cancel_elsewhere(app: &Arc<App>, external_id: &str) -> Result<Sent, Held> {
    if !super::orders_live(app) {
        return Err(Held::Dry);
    }
    Ok(broker(app).cancel(app, external_id))
}

/// Change an order placed elsewhere, as `cancel_elsewhere`.
pub fn modify_elsewhere(app: &Arc<App>, external_id: &str, limit_price: Option<Dec>, quantity: Option<Dec>) -> Result<Sent, Held> {
    if !super::orders_live(app) {
        return Err(Held::Dry);
    }
    let mut change = serde_json::Map::new();
    if let Some(p) = limit_price {
        change.insert("newLimitPrice".into(), json!(p.to_f64()));
    }
    if let Some(q) = quantity {
        change.insert("newQuantity".into(), json!(q.to_f64()));
    }
    Ok(broker(app).modify(app, external_id, &Value::Object(change)))
}
