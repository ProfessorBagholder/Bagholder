//! The orders Bagholder sends and the brackets that guard their fills, each with its
//! log (`docs/architecture.md` §11, `docs/plans/stage-4-execution.md`).
//!
//! An order or a bracket is written once; everything after is an event appended to
//! its log under one transaction, with who asked. Its state is the fold of the log
//! (`bagholder_core::order`, `bagholder_core::bracket`); an event that is not a move
//! from where it falls is kept with why it was refused and changes nothing. The
//! state columns are that fold, kept so live ones are found by state, never by a
//! list cut at a count.

use bagholder_core::bracket::{Bracket, BracketEvent, ExitRole, Phase, StopLeg, Trail};
use bagholder_core::order::{Applied, Asker, BrokerStatus, NotAllowed, OrderEvent, OrderFold, OrderKind, OrderRole, OrderState, Reading, Side, TimeInForce};
use bagholder_core::{Currency, Dec};
use rusqlite::{params, OptionalExtension, Row};
use serde_json::{json, Map, Value};

use crate::text::{self, at as at_text};
use crate::{Book, BookError, Result};

/// An order as it is written, before anything is sent.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderRequest {
    /// The external id the broker is sent and the order is read back by.
    pub id: String,
    pub broker: String,
    pub broker_account: String,
    pub broker_security: String,
    pub symbol: String,
    pub currency: Currency,
    pub side: Side,
    pub kind: OrderKind,
    pub quantity: Dec,
    pub limit_price: Option<Dec>,
    pub stop_price: Option<Dec>,
    pub time_in_force: TimeInForce,
    /// The bracket it is the entry or an exit of.
    pub bracket: Option<(String, OrderRole)>,
    /// The request as it is sent.
    pub request: Value,
}

/// An order as the book holds it: what was asked, and its fold.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredOrder {
    pub request: OrderRequest,
    pub created_at: jiff::Timestamp,
    pub fold: OrderFold,
    /// What the broker last stated of it.
    pub stated_price: Option<Dec>,
    pub stated_quantity: Option<Dec>,
    pub expires_at: Option<jiff::Timestamp>,
    pub updated_at: jiff::Timestamp,
}

/// One entry of a log: when, who asked, what, and why it was refused if it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Logged<E> {
    pub seq: i64,
    pub at: jiff::Timestamp,
    pub asker: Asker,
    pub event: E,
    pub refused: Option<String>,
}

/// Where a bracket's exits trade.
#[derive(Clone, Debug, PartialEq)]
pub struct BracketPlace {
    pub id: String,
    pub broker: String,
    pub broker_account: String,
    pub broker_security: String,
    pub symbol: String,
    pub currency: Currency,
}

/// A bracket as the book holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredBracket {
    pub place: BracketPlace,
    pub created_at: jiff::Timestamp,
    pub bracket: Bracket,
    pub updated_at: jiff::Timestamp,
}

const ORDER_COLUMNS: &str = "id, broker, broker_account, broker_security, symbol, currency, side, order_type, quantity, limit_price, stop_price, time_in_force, bracket_id, role, request, created_at, stated_price, stated_quantity, expires_at, updated_at";
const BRACKET_COLUMNS: &str = "id, broker, broker_account, broker_security, symbol, currency, created_at, updated_at";

impl Book {
    /// Write an order before anything is sent: `dry` when orders are off.
    pub fn write_order(&self, o: &OrderRequest, dry: bool, asker: &Asker, at: jiff::Timestamp) -> Result<()> {
        if !o.quantity.is_positive() {
            return Err(BookError::Refused(format!("an order for {} is not an order", o.quantity)));
        }
        let first = OrderEvent::Written { dry };
        let fold = OrderFold::start(&first).map_err(|e| BookError::Refused(e.to_string()))?;
        self.atomically(|| {
            if let Some((bracket, _)) = &o.bracket {
                let known: Option<String> = self.conn().query_row("SELECT id FROM brackets WHERE id = ?", [bracket], |r| r.get(0)).optional()?;
                if known.is_none() {
                    return Err(BookError::Refused(format!("no bracket {bracket}")));
                }
            }
            self.conn().execute(
                "INSERT INTO orders (id, broker, broker_account, broker_security, symbol, currency, side, order_type, quantity, limit_price, stop_price, time_in_force, bracket_id, role, request, created_at, state, filled, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?16)",
                params![
                    o.id, o.broker, o.broker_account, o.broker_security, o.symbol, o.currency.as_str(), o.side.as_str(), o.kind.as_str(), o.quantity.to_text(),
                    o.limit_price.map(Dec::to_text), o.stop_price.map(Dec::to_text), o.time_in_force.as_str(),
                    o.bracket.as_ref().map(|(b, _)| b.clone()), o.bracket.as_ref().map(|(_, r)| r.as_str()),
                    o.request.to_string(), at_text(at), fold.state.as_str(), fold.filled.to_text(),
                ],
            )?;
            self.append("order_events", "order_id", &o.id, 0, at, asker, first.kind(), &order_body(&first), None)?;
            Ok(())
        })
    }

    /// Record what happened to an order: the move it made, or why it is not one
    /// (kept in the log all the same, changing nothing).
    pub fn order_event(&self, id: &str, asker: &Asker, at: jiff::Timestamp, e: &OrderEvent) -> Result<std::result::Result<Applied, NotAllowed>> {
        self.atomically(|| {
            let log = self.order_log(id)?;
            if log.is_empty() {
                return Err(BookError::Refused(format!("no order {id}")));
            }
            let events: Vec<OrderEvent> = log.iter().map(|l| l.event.clone()).collect();
            let mut fold = OrderFold::of(&events).ok_or_else(|| text::corrupt("order_events", "kind", id, "a log that does not start with the order written"))?;
            let applied = fold.apply(e);
            let seq = log.last().map_or(0, |l| l.seq) + 1;
            self.append("order_events", "order_id", id, seq, at, asker, e.kind(), &order_body(e), applied.as_ref().err().map(|n| n.to_string()))?;
            if applied.is_ok() {
                let stated = match e {
                    OrderEvent::Read(r) => Some(r),
                    _ => None,
                };
                self.conn().execute(
                    "UPDATE orders SET state = ?2, broker_id = ?3, filled = ?4, average = ?5, why = ?6, code = ?7, updated_at = ?8,
                        stated_price = COALESCE(?9, stated_price), stated_quantity = COALESCE(?10, stated_quantity), expires_at = COALESCE(?11, expires_at)
                     WHERE id = ?1",
                    params![
                        id, fold.state.as_str(), fold.broker_id, fold.filled.to_text(), fold.average.map(Dec::to_text), fold.why, fold.code, at_text(at),
                        stated.and_then(|r| r.price).map(Dec::to_text), stated.and_then(|r| r.quantity).map(Dec::to_text), stated.and_then(|r| r.expires_at).map(at_text),
                    ],
                )?;
            }
            Ok(applied)
        })
    }

    /// An order by its id.
    pub fn order(&self, id: &str) -> Result<Option<StoredOrder>> {
        let row = self.conn().query_row(&format!("SELECT {ORDER_COLUMNS} FROM orders WHERE id = ?"), [id], order_columns).optional()?;
        row.map(|r| self.stored_order(r)).transpose()
    }

    /// Every order the broker may still act on, however many and however old.
    pub fn orders_in_flight(&self) -> Result<Vec<StoredOrder>> {
        self.orders_where("state IN ('sending','unconfirmed','pending','partly-filled','cancelling')", &[])
    }

    /// Every order of a bracket: its entry and each exit it placed.
    pub fn orders_of_bracket(&self, bracket: &str) -> Result<Vec<StoredOrder>> {
        self.orders_where("bracket_id = ?", &[bracket])
    }

    /// Orders written before `before` (all when `None`), newest first, `limit` of
    /// them: the page's own paging of the list, never what the engine acts on.
    pub fn orders_before(&self, before: Option<jiff::Timestamp>, limit: u32) -> Result<Vec<StoredOrder>> {
        let rows = {
            let mut s = self.conn().prepare_cached(&format!("SELECT {ORDER_COLUMNS} FROM orders WHERE (?1 IS NULL OR created_at < ?1) ORDER BY created_at DESC, id DESC LIMIT ?2"))?;
            let rows = s.query_map(params![before.map(at_text), limit], order_columns)?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        rows.into_iter().map(|r| self.stored_order(r)).collect()
    }

    fn orders_where(&self, clause: &str, args: &[&str]) -> Result<Vec<StoredOrder>> {
        let rows = {
            let mut s = self.conn().prepare_cached(&format!("SELECT {ORDER_COLUMNS} FROM orders WHERE {clause} ORDER BY created_at, id"))?;
            let rows = s.query_map(rusqlite::params_from_iter(args), order_columns)?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        rows.into_iter().map(|r| self.stored_order(r)).collect()
    }

    /// An order's log, in order.
    pub fn order_log(&self, id: &str) -> Result<Vec<Logged<OrderEvent>>> {
        self.log("order_events", "order_id", id, |kind, body| order_event_of(kind, body))
    }

    fn stored_order(&self, r: OrderColumns) -> Result<StoredOrder> {
        const T: &str = "orders";
        let log = self.order_log(&r.id)?;
        let events: Vec<OrderEvent> = log.into_iter().map(|l| l.event).collect();
        let fold = OrderFold::of(&events).ok_or_else(|| text::corrupt("order_events", "kind", &r.id, "a log that does not start with the order written"))?;
        let bracket = match (r.bracket_id, r.role) {
            (Some(b), Some(role)) => Some((b, text::parsed(T, "role", &role, OrderRole::parse)?)),
            (None, None) => None,
            _ => return Err(text::corrupt(T, "role", &r.id, "a bracket without its role")),
        };
        Ok(StoredOrder {
            request: OrderRequest {
                broker: r.broker,
                broker_account: r.broker_account,
                broker_security: r.broker_security,
                symbol: r.symbol,
                currency: text::currency(T, "currency", &r.currency)?,
                side: text::parsed(T, "side", &r.side, Side::parse)?,
                kind: text::parsed(T, "order_type", &r.order_type, OrderKind::parse)?,
                quantity: text::dec(T, "quantity", &r.quantity)?,
                limit_price: text::opt_dec(T, "limit_price", r.limit_price)?,
                stop_price: text::opt_dec(T, "stop_price", r.stop_price)?,
                time_in_force: text::parsed(T, "time_in_force", &r.time_in_force, TimeInForce::parse)?,
                bracket,
                request: serde_json::from_str(&r.request).map_err(|e| text::corrupt(T, "request", &r.request, e))?,
                id: r.id,
            },
            created_at: text::instant(T, "created_at", &r.created_at)?,
            fold,
            stated_price: text::opt_dec(T, "stated_price", r.stated_price)?,
            stated_quantity: text::opt_dec(T, "stated_quantity", r.stated_quantity)?,
            expires_at: text::opt_instant(T, "expires_at", r.expires_at)?,
            updated_at: text::instant(T, "updated_at", &r.updated_at)?,
        })
    }

    /// Write a bracket, with its first event.
    pub fn write_bracket(&self, place: &BracketPlace, created: &BracketEvent, asker: &Asker, at: jiff::Timestamp) -> Result<()> {
        let BracketEvent::Created { .. } = created else {
            return Err(BookError::Refused("a bracket starts by being created".into()));
        };
        self.atomically(|| {
            self.conn().execute(
                "INSERT INTO brackets (id, broker, broker_account, broker_security, symbol, currency, created_at, phase, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'waiting', ?7)",
                params![place.id, place.broker, place.broker_account, place.broker_security, place.symbol, place.currency.as_str(), at_text(at)],
            )?;
            self.append("bracket_events", "bracket_id", &place.id, 0, at, asker, created.kind(), &bracket_body(created), None)?;
            Ok(())
        })
    }

    /// Record what happened to a bracket: `Err` with why when it is not a move from
    /// where it stands (kept in the log all the same, changing nothing).
    pub fn bracket_event(&self, id: &str, asker: &Asker, at: jiff::Timestamp, e: &BracketEvent) -> Result<std::result::Result<Phase, String>> {
        self.atomically(|| {
            let log = self.bracket_log(id)?;
            let mut b = fold_bracket(id, &log)?;
            let applied = b.apply(at, e);
            let seq = log.last().map_or(0, |l| l.seq) + 1;
            self.append("bracket_events", "bracket_id", id, seq, at, asker, e.kind(), &bracket_body(e), applied.as_ref().err().cloned())?;
            if applied.is_ok() {
                self.conn().execute("UPDATE brackets SET phase = ?2, updated_at = ?3 WHERE id = ?1", params![id, b.phase.as_str(), at_text(at)])?;
            }
            Ok(applied.map(|_| b.phase))
        })
    }

    pub fn bracket(&self, id: &str) -> Result<Option<StoredBracket>> {
        let row = self.conn().query_row(&format!("SELECT {BRACKET_COLUMNS} FROM brackets WHERE id = ?"), [id], bracket_columns).optional()?;
        row.map(|r| self.stored_bracket(r)).transpose()
    }

    /// Every bracket that is not ended, however many.
    pub fn live_brackets(&self) -> Result<Vec<StoredBracket>> {
        self.brackets_where("phase <> 'ended'", None)
    }

    /// Brackets created before `before`, newest first, `limit` of them: the page's paging.
    pub fn brackets_before(&self, before: Option<jiff::Timestamp>, limit: u32) -> Result<Vec<StoredBracket>> {
        self.brackets_where("(?1 IS NULL OR created_at < ?1)", Some((before, limit)))
    }

    fn brackets_where(&self, clause: &str, page: Option<(Option<jiff::Timestamp>, u32)>) -> Result<Vec<StoredBracket>> {
        let rows = {
            let rows = match page {
                None => {
                    let mut s = self.conn().prepare_cached(&format!("SELECT {BRACKET_COLUMNS} FROM brackets WHERE {clause} ORDER BY created_at, id"))?;
                    let rows = s.query_map([], bracket_columns)?.collect::<rusqlite::Result<Vec<_>>>()?;
                    rows
                }
                Some((before, limit)) => {
                    let mut s = self.conn().prepare_cached(&format!("SELECT {BRACKET_COLUMNS} FROM brackets WHERE {clause} ORDER BY created_at DESC, id DESC LIMIT ?2"))?;
                    let rows = s.query_map(params![before.map(at_text), limit], bracket_columns)?.collect::<rusqlite::Result<Vec<_>>>()?;
                    rows
                }
            };
            rows
        };
        rows.into_iter().map(|r| self.stored_bracket(r)).collect()
    }

    /// A bracket's log, in order.
    pub fn bracket_log(&self, id: &str) -> Result<Vec<Logged<BracketEvent>>> {
        self.log("bracket_events", "bracket_id", id, bracket_event_of)
    }

    fn stored_bracket(&self, r: BracketColumns) -> Result<StoredBracket> {
        const T: &str = "brackets";
        let log = self.bracket_log(&r.id)?;
        let bracket = fold_bracket(&r.id, &log)?;
        Ok(StoredBracket {
            place: BracketPlace { currency: text::currency(T, "currency", &r.currency)?, id: r.id, broker: r.broker, broker_account: r.broker_account, broker_security: r.broker_security, symbol: r.symbol },
            created_at: text::instant(T, "created_at", &r.created_at)?,
            bracket,
            updated_at: text::instant(T, "updated_at", &r.updated_at)?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn append(&self, table: &str, key: &str, id: &str, seq: i64, at: jiff::Timestamp, asker: &Asker, kind: &str, body: &Value, refused: Option<String>) -> Result<()> {
        self.conn().execute(
            &format!("INSERT INTO {table} ({key}, seq, at, asker, kind, body, refused) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
            params![id, seq, at_text(at), asker.to_text(), kind, body.to_string(), refused],
        )?;
        Ok(())
    }

    fn log<E>(&self, table: &'static str, key: &str, id: &str, read: impl Fn(&str, &Map<String, Value>) -> std::result::Result<E, String>) -> Result<Vec<Logged<E>>> {
        let rows: Vec<(i64, String, String, String, String, Option<String>)> = {
            let mut s = self.conn().prepare_cached(&format!("SELECT seq, at, asker, kind, body, refused FROM {table} WHERE {key} = ? ORDER BY seq"))?;
            let rows = s.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        rows.into_iter()
            .map(|(seq, at, asker, kind, body, refused)| {
                let map: Map<String, Value> = serde_json::from_str(&body).map_err(|e| text::corrupt(table, "body", &body, e))?;
                Ok(Logged {
                    seq,
                    at: text::instant(table, "at", &at)?,
                    asker: text::parsed(table, "asker", &asker, Asker::parse)?,
                    event: read(&kind, &map).map_err(|why| text::corrupt(table, "body", &body, why))?,
                    refused,
                })
            })
            .collect()
    }
}

fn fold_bracket(id: &str, log: &[Logged<BracketEvent>]) -> Result<Bracket> {
    let events: Vec<(jiff::Timestamp, BracketEvent)> = log.iter().map(|l| (l.at, l.event.clone())).collect();
    Bracket::of(&events).ok_or_else(|| text::corrupt("bracket_events", "kind", id, "a log that does not start with the bracket created"))
}

struct OrderColumns {
    id: String,
    broker: String,
    broker_account: String,
    broker_security: String,
    symbol: String,
    currency: String,
    side: String,
    order_type: String,
    quantity: String,
    limit_price: Option<String>,
    stop_price: Option<String>,
    time_in_force: String,
    bracket_id: Option<String>,
    role: Option<String>,
    request: String,
    created_at: String,
    stated_price: Option<String>,
    stated_quantity: Option<String>,
    expires_at: Option<String>,
    updated_at: String,
}

fn order_columns(r: &Row) -> rusqlite::Result<OrderColumns> {
    Ok(OrderColumns {
        id: r.get(0)?,
        broker: r.get(1)?,
        broker_account: r.get(2)?,
        broker_security: r.get(3)?,
        symbol: r.get(4)?,
        currency: r.get(5)?,
        side: r.get(6)?,
        order_type: r.get(7)?,
        quantity: r.get(8)?,
        limit_price: r.get(9)?,
        stop_price: r.get(10)?,
        time_in_force: r.get(11)?,
        bracket_id: r.get(12)?,
        role: r.get(13)?,
        request: r.get(14)?,
        created_at: r.get(15)?,
        stated_price: r.get(16)?,
        stated_quantity: r.get(17)?,
        expires_at: r.get(18)?,
        updated_at: r.get(19)?,
    })
}

struct BracketColumns {
    id: String,
    broker: String,
    broker_account: String,
    broker_security: String,
    symbol: String,
    currency: String,
    created_at: String,
    updated_at: String,
}

fn bracket_columns(r: &Row) -> rusqlite::Result<BracketColumns> {
    Ok(BracketColumns { id: r.get(0)?, broker: r.get(1)?, broker_account: r.get(2)?, broker_security: r.get(3)?, symbol: r.get(4)?, currency: r.get(5)?, created_at: r.get(6)?, updated_at: r.get(7)? })
}

// ---------------------------------------------------------------------------
// the events as kept: one JSON object each, read back strictly
// ---------------------------------------------------------------------------

fn dec_v(d: Dec) -> Value {
    Value::String(d.to_text())
}

fn opt_dec_v(d: Option<Dec>) -> Value {
    d.map_or(Value::Null, dec_v)
}

fn status_word(s: BrokerStatus) -> &'static str {
    match s {
        BrokerStatus::Open => "open",
        BrokerStatus::Filled => "filled",
        BrokerStatus::Cancelled => "cancelled",
        BrokerStatus::Expired => "expired",
        BrokerStatus::Rejected => "rejected",
        BrokerStatus::NotFound => "not-found",
    }
}

fn order_body(e: &OrderEvent) -> Value {
    match e {
        OrderEvent::Written { dry } => json!({ "dry": dry }),
        OrderEvent::NotSent { why } | OrderEvent::Unclear { why } | OrderEvent::CancelRefused { why } => json!({ "why": why }),
        OrderEvent::Accepted { broker_id } => json!({ "broker_id": broker_id }),
        OrderEvent::Refused { why, code } => json!({ "why": why, "code": code }),
        OrderEvent::Read(r) => json!({
            "status": status_word(r.status), "filled": dec_v(r.filled), "average": opt_dec_v(r.average),
            "price": opt_dec_v(r.price), "quantity": opt_dec_v(r.quantity), "expires_at": r.expires_at.map(at_text),
        }),
        OrderEvent::CancelAsked => json!({}),
        OrderEvent::ModifyAsked { limit_price, quantity } => json!({ "limit_price": opt_dec_v(*limit_price), "quantity": opt_dec_v(*quantity) }),
        OrderEvent::ModifyRefused { why } => json!({ "why": why }),
    }
}

/// A field read strictly: present, of the type asked.
struct Fields<'a>(&'a Map<String, Value>);

impl Fields<'_> {
    fn only(&self, names: &[&str]) -> std::result::Result<(), String> {
        match self.0.keys().find(|k| !names.contains(&k.as_str())) {
            Some(k) => Err(format!("a field {k:?} this build does not know")),
            None => Ok(()),
        }
    }

    fn text(&self, name: &str) -> std::result::Result<String, String> {
        self.0.get(name).and_then(Value::as_str).map(String::from).ok_or_else(|| format!("no text {name:?}"))
    }

    fn opt_text(&self, name: &str) -> std::result::Result<Option<String>, String> {
        match self.0.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(v) => Err(format!("{name:?} is {v}, not text")),
        }
    }

    fn bool(&self, name: &str) -> std::result::Result<bool, String> {
        self.0.get(name).and_then(Value::as_bool).ok_or_else(|| format!("no true or false {name:?}"))
    }

    fn dec(&self, name: &str) -> std::result::Result<Dec, String> {
        let t = self.text(name)?;
        let d = Dec::parse(&t).map_err(|e| e.to_string())?;
        if d.to_text() != t {
            return Err(format!("{name:?} {t:?} is not in the form the book writes"));
        }
        Ok(d)
    }

    fn opt_dec(&self, name: &str) -> std::result::Result<Option<Dec>, String> {
        match self.opt_text(name)? {
            None => Ok(None),
            Some(_) => self.dec(name).map(Some),
        }
    }

    fn opt_instant(&self, name: &str) -> std::result::Result<Option<jiff::Timestamp>, String> {
        self.opt_text(name)?.map(|t| t.parse().map_err(|e: jiff::Error| e.to_string())).transpose()
    }

    fn word<T, E: ToString>(&self, name: &str, parse: impl FnOnce(&str) -> std::result::Result<T, E>) -> std::result::Result<T, String> {
        parse(&self.text(name)?).map_err(|e| e.to_string())
    }
}

fn order_event_of(kind: &str, m: &Map<String, Value>) -> std::result::Result<OrderEvent, String> {
    let f = Fields(m);
    Ok(match kind {
        "written" => {
            f.only(&["dry"])?;
            OrderEvent::Written { dry: f.bool("dry")? }
        }
        "not-sent" => {
            f.only(&["why"])?;
            OrderEvent::NotSent { why: f.text("why")? }
        }
        "unclear" => {
            f.only(&["why"])?;
            OrderEvent::Unclear { why: f.text("why")? }
        }
        "cancel-refused" => {
            f.only(&["why"])?;
            OrderEvent::CancelRefused { why: f.text("why")? }
        }
        "accepted" => {
            f.only(&["broker_id"])?;
            OrderEvent::Accepted { broker_id: f.text("broker_id")? }
        }
        "refused" => {
            f.only(&["why", "code"])?;
            OrderEvent::Refused { why: f.text("why")?, code: f.opt_text("code")? }
        }
        "read" => {
            f.only(&["status", "filled", "average", "price", "quantity", "expires_at"])?;
            let status = match f.text("status")?.as_str() {
                "open" => BrokerStatus::Open,
                "filled" => BrokerStatus::Filled,
                "cancelled" => BrokerStatus::Cancelled,
                "expired" => BrokerStatus::Expired,
                "rejected" => BrokerStatus::Rejected,
                "not-found" => BrokerStatus::NotFound,
                other => return Err(format!("not a broker status: {other:?}")),
            };
            OrderEvent::Read(Reading { status, filled: f.dec("filled")?, average: f.opt_dec("average")?, price: f.opt_dec("price")?, quantity: f.opt_dec("quantity")?, expires_at: f.opt_instant("expires_at")? })
        }
        "cancel-asked" => {
            f.only(&[])?;
            OrderEvent::CancelAsked
        }
        "modify-asked" => {
            f.only(&["limit_price", "quantity"])?;
            OrderEvent::ModifyAsked { limit_price: f.opt_dec("limit_price")?, quantity: f.opt_dec("quantity")? }
        }
        "modify-refused" => {
            f.only(&["why"])?;
            OrderEvent::ModifyRefused { why: f.text("why")? }
        }
        other => return Err(format!("not an order event: {other:?}")),
    })
}

fn stop_v(s: &Option<StopLeg>) -> Value {
    match s {
        None => Value::Null,
        Some(s) => json!({
            "level": dec_v(s.level),
            "trail_pct": match s.trail { Some(Trail::Pct(p)) => dec_v(p), _ => Value::Null },
            "trail_amount": match s.trail { Some(Trail::Amount(a)) => dec_v(a), _ => Value::Null },
            "high": opt_dec_v(s.high),
        }),
    }
}

fn stop_of(v: Option<&Value>) -> std::result::Result<Option<StopLeg>, String> {
    let m = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(m)) => m,
        Some(other) => return Err(format!("a stop leg that is {other}")),
    };
    let f = Fields(m);
    f.only(&["level", "trail_pct", "trail_amount", "high"])?;
    let trail = match (f.opt_dec("trail_pct")?, f.opt_dec("trail_amount")?) {
        (None, None) => None,
        (Some(p), None) => Some(Trail::Pct(p)),
        (None, Some(a)) => Some(Trail::Amount(a)),
        _ => return Err("a stop that trails by both a percent and an amount".into()),
    };
    Ok(Some(StopLeg { level: f.dec("level")?, trail, high: f.opt_dec("high")? }))
}

fn bracket_body(e: &BracketEvent) -> Value {
    match e {
        BracketEvent::Created { quantity, stop, target } => json!({ "quantity": dec_v(*quantity), "stop": stop_v(stop), "target": opt_dec_v(*target) }),
        BracketEvent::Armed { quantity, high, native } => json!({ "quantity": dec_v(*quantity), "high": opt_dec_v(*high), "native": native }),
        BracketEvent::EntryEnded { why } | BracketEvent::Halted { why } | BracketEvent::SaleDropped { why } => json!({ "why": why }),
        BracketEvent::Trailed { level, high } => json!({ "level": dec_v(*level), "high": dec_v(*high) }),
        BracketEvent::Adopted { level, target, quantity } => json!({ "level": opt_dec_v(*level), "target": opt_dec_v(*target), "quantity": opt_dec_v(*quantity) }),
        BracketEvent::Adjusted { stop, target } => json!({ "stop": stop_v(stop), "target": opt_dec_v(*target) }),
        BracketEvent::Placed { role, order_id, price, quantity } => json!({ "role": role.as_str(), "order_id": order_id, "price": opt_dec_v(*price), "quantity": dec_v(*quantity) }),
        BracketEvent::Refused { why, code } => json!({ "why": why, "code": code }),
        BracketEvent::CancelAsked { order_id } => json!({ "order_id": order_id }),
        BracketEvent::Cleared { filled } => json!({ "filled": dec_v(*filled) }),
        BracketEvent::Moved { to } => json!({ "to": to.as_str() }),
        BracketEvent::Ended { outcome } => json!({ "outcome": outcome }),
        BracketEvent::Done => json!({}),
        BracketEvent::SaleAsked { quantity } | BracketEvent::Sold { quantity } => json!({ "quantity": dec_v(*quantity) }),
        BracketEvent::PositionRead { held, read_at } => json!({ "held": held, "read_at": at_text(*read_at) }),
    }
}

fn bracket_event_of(kind: &str, m: &Map<String, Value>) -> std::result::Result<BracketEvent, String> {
    let f = Fields(m);
    Ok(match kind {
        "created" => {
            f.only(&["quantity", "stop", "target"])?;
            BracketEvent::Created { quantity: f.dec("quantity")?, stop: stop_of(m.get("stop"))?, target: f.opt_dec("target")? }
        }
        "armed" => {
            f.only(&["quantity", "high", "native"])?;
            BracketEvent::Armed { quantity: f.dec("quantity")?, high: f.opt_dec("high")?, native: f.bool("native")? }
        }
        "entry-ended" => {
            f.only(&["why"])?;
            BracketEvent::EntryEnded { why: f.text("why")? }
        }
        "halted" => {
            f.only(&["why"])?;
            BracketEvent::Halted { why: f.text("why")? }
        }
        "sale-dropped" => {
            f.only(&["why"])?;
            BracketEvent::SaleDropped { why: f.text("why")? }
        }
        "trailed" => {
            f.only(&["level", "high"])?;
            BracketEvent::Trailed { level: f.dec("level")?, high: f.dec("high")? }
        }
        "adopted" => {
            f.only(&["level", "target", "quantity"])?;
            BracketEvent::Adopted { level: f.opt_dec("level")?, target: f.opt_dec("target")?, quantity: f.opt_dec("quantity")? }
        }
        "adjusted" => {
            f.only(&["stop", "target"])?;
            BracketEvent::Adjusted { stop: stop_of(m.get("stop"))?, target: f.opt_dec("target")? }
        }
        "placed" => {
            f.only(&["role", "order_id", "price", "quantity"])?;
            BracketEvent::Placed { role: f.word("role", ExitRole::parse)?, order_id: f.text("order_id")?, price: f.opt_dec("price")?, quantity: f.dec("quantity")? }
        }
        "refused" => {
            f.only(&["why", "code"])?;
            BracketEvent::Refused { why: f.text("why")?, code: f.opt_text("code")? }
        }
        "cancel-asked" => {
            f.only(&["order_id"])?;
            BracketEvent::CancelAsked { order_id: f.text("order_id")? }
        }
        "cleared" => {
            f.only(&["filled"])?;
            BracketEvent::Cleared { filled: f.dec("filled")? }
        }
        "moved" => {
            f.only(&["to"])?;
            BracketEvent::Moved { to: f.word("to", Phase::parse)? }
        }
        "ended" => {
            f.only(&["outcome"])?;
            BracketEvent::Ended { outcome: f.text("outcome")? }
        }
        "done" => {
            f.only(&[])?;
            BracketEvent::Done
        }
        "sale-asked" => {
            f.only(&["quantity"])?;
            BracketEvent::SaleAsked { quantity: f.dec("quantity")? }
        }
        "sold" => {
            f.only(&["quantity"])?;
            BracketEvent::Sold { quantity: f.dec("quantity")? }
        }
        "position-read" => {
            f.only(&["held", "read_at"])?;
            BracketEvent::PositionRead { held: f.bool("held")?, read_at: f.opt_instant("read_at")?.ok_or("no read_at")? }
        }
        other => return Err(format!("not a bracket event: {other:?}")),
    })
}

/// The state words the book keeps, as the order fold names them (for a test to hold
/// the table's check constraint to the fold's states).
pub fn order_state_words() -> Vec<&'static str> {
    OrderState::ALL.iter().map(|s| s.as_str()).collect()
}

pub fn bracket_phase_words() -> Vec<&'static str> {
    Phase::ALL.iter().map(|p| p.as_str()).collect()
}
