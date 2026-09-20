//! The orders store spoken to in JSON, for the callers not yet moved to `typed`
//! (docs/architecture.md, stage 5). Every function here is the typed one with a row
//! turned into JSON, or out of it, at the edge: there is one reading of a column and
//! one writing of it, in `typed`.

use rusqlite::{Connection, Result};
use serde_json::{Map, Value};

use super::typed;
use super::types::*;

fn to_json<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn from_json<T: serde::de::DeserializeOwned + Default>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap_or_default()
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn num_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

impl OrderPatch {
    /// The patch a JSON object names: a key that is there is set, one that is not is left.
    pub fn from_json(patch: &Value) -> OrderPatch {
        let mut p = OrderPatch::default();
        let Some(m) = patch.as_object() else { return p };
        for (k, v) in m {
            match k.as_str() {
                "status" => p.status = Some(OrderStatus::parse(&text_of(v))),
                "wsOrderId" => p.ws_order_id = Some(text_of(v)),
                "error" => p.error = Some(text_of(v)),
                "wsStatus" => p.ws_status = Some(text_of(v)),
                "submittedAt" => p.submitted_at = Some(text_of(v)),
                "expiresAt" => p.expires_at = Some(text_of(v)),
                "tif" => p.tif = Some(text_of(v)),
                "currency" => p.currency = Some(text_of(v)),
                "symbol" => p.symbol = Some(text_of(v)),
                "filledQty" => p.filled_qty = Some(num_of(v)),
                "avgFill" => p.avg_fill = Some(num_of(v)),
                "quantity" => p.quantity = Some(num_of(v)),
                "limitPrice" => p.limit_price = Some(num_of(v)),
                "stopPrice" => p.stop_price = Some(num_of(v)),
                _ => {}
            }
        }
        p
    }
}

impl BracketPatch {
    /// As `OrderPatch::from_json`.
    pub fn from_json(patch: &Value) -> BracketPatch {
        let mut p = BracketPatch::default();
        let Some(m) = patch.as_object() else { return p };
        for (k, v) in m {
            match k.as_str() {
                "symbol" => p.symbol = Some(text_of(v)),
                "currency" => p.currency = Some(text_of(v)),
                "tif" => p.tif = Some(text_of(v)),
                "slKind" => p.sl_kind = Some(SlKind::parse(&text_of(v))),
                "slTrailUnit" => p.sl_trail_unit = Some(TrailUnit::parse(&text_of(v))),
                "slOrderId" => p.sl_order_id = Some(text_of(v)),
                "tpOrderId" => p.tp_order_id = Some(text_of(v)),
                "status" => p.status = Some(BracketStatus::parse(&text_of(v))),
                "outcome" => p.outcome = Some(text_of(v)),
                "error" => p.error = Some(text_of(v)),
                "movedAt" => p.moved_at = Some(text_of(v)),
                "armedAt" => p.armed_at = Some(text_of(v)),
                "slMode" => p.sl_mode = Some(SlMode::parse(&text_of(v))),
                "missedAt" => p.missed_at = Some(text_of(v)),
                "quantity" => p.quantity = Some(num_of(v)),
                "slPrice" => p.sl_price = Some(num_of(v)),
                "slTrail" => p.sl_trail = Some(num_of(v)),
                "highWater" => p.high_water = Some(num_of(v)),
                "tpPrice" => p.tp_price = Some(num_of(v)),
                "attempts" => p.attempts = Some(num_of(v).unwrap_or(0.0) as i64),
                "slNative" => p.sl_native = Some(truthy(v)),
                "seenHeld" => p.seen_held = Some(truthy(v)),
                _ => {}
            }
        }
        p
    }
}

pub fn insert_order(conn: &Connection, row: &Value, now: &str) -> Result<()> {
    typed::insert_order(conn, &from_json::<Order>(row), now)
}

pub fn update_order(conn: &Connection, order_id: &str, patch: &Value, now: &str) -> Result<()> {
    typed::update_order(conn, order_id, &OrderPatch::from_json(patch), now)
}

pub fn list_orders(conn: &Connection, limit: i64) -> Result<Vec<Value>> {
    Ok(typed::list_orders(conn, limit)?.iter().map(to_json).collect())
}

pub fn get_order(conn: &Connection, order_id: &str) -> Result<Option<Value>> {
    Ok(typed::get_order(conn, order_id)?.as_ref().map(to_json))
}

/// Record that this order's fill has been written as a local activity, so a later
/// status poll does not book it twice.
///
/// The booked quantity only ever grows; a smaller value never lowers it. The answer is
/// whether this statement changed a row, not the pooled connection's running tally.
pub fn mark_order_fill_booked(conn: &Connection, order_id: &str, qty: f64, now: &str) -> Result<bool> {
    let n = conn.execute(
        "UPDATE orders SET fill_booked_qty = ?, updated_at = ? WHERE id = ? AND (fill_booked_qty IS NULL OR fill_booked_qty < ?)",
        rusqlite::params![qty, now, order_id, qty],
    )?;
    Ok(n > 0)
}

pub fn insert_bracket(conn: &Connection, b: &Value, now: &str) -> Result<()> {
    typed::insert_bracket(conn, &from_json::<Bracket>(b), now)
}

pub fn update_bracket(conn: &Connection, bracket_id: &str, patch: &Value, now: &str) -> Result<()> {
    typed::update_bracket(conn, bracket_id, &BracketPatch::from_json(patch), now)
}

pub fn list_brackets(conn: &Connection, statuses: &[String]) -> Result<Vec<Value>> {
    let of: Vec<BracketStatus> = statuses.iter().map(|s| BracketStatus::parse(s)).collect();
    Ok(typed::list_brackets(conn, &of)?.iter().map(to_json).collect())
}

pub fn get_bracket(conn: &Connection, bracket_id: &str) -> Result<Option<Value>> {
    Ok(typed::get_bracket(conn, bracket_id)?.as_ref().map(to_json))
}

pub fn bracket_for_order(conn: &Connection, order_id: &str) -> Result<Option<Value>> {
    Ok(typed::bracket_for_order(conn, order_id)?.as_ref().map(to_json))
}

/// The symbol the book uses for a security, from its activity rows.
///
/// For an option that is the contract name, which the securities table does
/// not carry. Empty when the book has no row for it.
pub fn symbol_for_security(conn: &Connection, security_id: &str) -> Result<String> {
    let sid = security_id.trim();
    if sid.is_empty() {
        return Ok(String::new());
    }
    let s: Option<String> = conn
        .query_row(
            "SELECT symbol FROM activities WHERE security_id = ? AND symbol IS NOT NULL AND symbol != '' ORDER BY COALESCE(occurred_at, transaction_date) DESC LIMIT 1",
            [sid],
            |r| r.get(0),
        )
        .unwrap_or(None);
    Ok(s.unwrap_or_default())
}

/// The open orders the header counts.
pub fn open_orders_count(conn: &Connection, open_statuses: &[&str]) -> Result<i64> {
    let of: Vec<OrderStatus> = open_statuses.iter().map(|s| OrderStatus::parse(s)).collect();
    typed::open_orders_count(conn, &of)
}

/// Kept for callers holding a patch as a map.
pub fn patch_of(m: Map<String, Value>) -> Value {
    Value::Object(m)
}
