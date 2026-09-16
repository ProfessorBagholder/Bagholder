//! Order tickets and the brackets that watch them.
//!
//! Every ticket is written before anything is sent, so a crash between the
//! write and the broker's answer leaves a record rather than a silence. A
//! ticket is never deleted; it gains a status.

use rusqlite::{Connection, Result, Row};
use serde_json::{json, Map, Value};

use bagholder_model::value::{field_s, get, num};

/// A number that is absent rather than zero.
fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { None } else { Some(n) }
        }
    }
}

fn text(r: &Row, name: &str) -> Result<String> {
    Ok(r.get::<_, Option<String>>(name)?.unwrap_or_default())
}

fn real(r: &Row, name: &str) -> Result<Value> {
    Ok(match r.get::<_, Option<f64>>(name)? { Some(v) => json!(v), None => Value::Null })
}

/// A JSON column, or nothing when it is empty or unreadable.
fn js(r: &Row, name: &str) -> Result<Value> {
    let raw: Option<String> = r.get(name)?;
    Ok(match raw {
        Some(s) if !s.is_empty() => serde_json::from_str(&s).unwrap_or(Value::Null),
        _ => Value::Null,
    })
}

/// `store._order_from_row`.
pub fn order_from_row(r: &Row) -> Result<Value> {
    let source = { let s = text(r, "source")?; if s.is_empty() { "bagholder".to_string() } else { s } };
    let role = { let s = text(r, "role")?; if s.is_empty() { "entry".to_string() } else { s } };
    Ok(json!({
        "id": text(r, "id")?,
        "createdAt": text(r, "created_at")?,
        "accountId": text(r, "account_id")?,
        "account": text(r, "account")?,
        "securityId": text(r, "security_id")?,
        "symbol": text(r, "symbol")?,
        "currency": text(r, "currency")?,
        "side": text(r, "side")?,
        "type": text(r, "type")?,
        "quantity": real(r, "quantity")?,
        "limitPrice": real(r, "limit_price")?,
        "stopPrice": real(r, "stop_price")?,
        "tif": text(r, "tif")?,
        "stopLoss": js(r, "stop_loss")?,
        "takeProfit": js(r, "take_profit")?,
        "status": text(r, "status")?,
        "wsOrderId": text(r, "ws_order_id")?,
        "error": text(r, "error")?,
        "request": js(r, "request")?,
        "updatedAt": text(r, "updated_at")?,
        "source": source,
        "wsStatus": text(r, "ws_status")?,
        "filledQty": real(r, "filled_qty")?,
        "avgFill": real(r, "avg_fill")?,
        "submittedAt": text(r, "submitted_at")?,
        "expiresAt": text(r, "expires_at")?,
        "parentId": text(r, "parent_id")?,
        "role": role,
        "fillBookedQty": real(r, "fill_booked_qty")?,
    }))
}

/// `store.insert_order`: a new ticket, written before anything is sent.
pub fn insert_order(conn: &Connection, row: &Value, now: &str) -> Result<()> {
    let created = { let c = field_s(row, "createdAt"); if c.is_empty() { now.to_string() } else { c } };
    let source = { let s = field_s(row, "source"); if s.is_empty() { "bagholder".to_string() } else { s } };
    let role = { let s = field_s(row, "role"); if s.is_empty() { "entry".to_string() } else { s } };
    // the request is stored with its keys sorted, so the same ticket is the
    // same text whichever order it was built in
    let request = get(row, "request").filter(|v| truthy(v)).map(crate::tables::py_json_sorted);
    let stop_loss = get(row, "stopLoss").filter(|v| truthy(v)).map(crate::tables::py_json);
    let take_profit = get(row, "takeProfit").filter(|v| truthy(v)).map(crate::tables::py_json);

    conn.execute(
        "INSERT INTO orders (id, created_at, account_id, account, security_id, symbol, currency, side, type, quantity, \
         limit_price, stop_price, tif, stop_loss, take_profit, status, ws_order_id, error, request, updated_at, \
         source, ws_status, filled_qty, avg_fill, submitted_at, expires_at, parent_id, role) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            field_s(row, "id"), created, field_s(row, "accountId"), field_s(row, "account"),
            field_s(row, "securityId"), field_s(row, "symbol"), field_s(row, "currency"),
            field_s(row, "side"), field_s(row, "type"), num(get(row, "quantity"), 0.0),
            opt_num(get(row, "limitPrice")), opt_num(get(row, "stopPrice")), field_s(row, "tif"),
            stop_loss, take_profit,
            field_s(row, "status"), field_s(row, "wsOrderId"), field_s(row, "error"),
            request, now,
            source, field_s(row, "wsStatus"), opt_num(get(row, "filledQty")), opt_num(get(row, "avgFill")),
            field_s(row, "submittedAt"), field_s(row, "expiresAt"), field_s(row, "parentId"), role,
        ],
    )?;
    Ok(())
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

const ORDER_TEXT: [(&str, &str); 9] = [
    ("status", "status"), ("wsOrderId", "ws_order_id"), ("error", "error"), ("wsStatus", "ws_status"),
    ("submittedAt", "submitted_at"), ("expiresAt", "expires_at"), ("tif", "tif"), ("currency", "currency"),
    ("symbol", "symbol"),
];
const ORDER_NUM: [(&str, &str); 5] = [
    ("filledQty", "filled_qty"), ("avgFill", "avg_fill"), ("quantity", "quantity"),
    ("limitPrice", "limit_price"), ("stopPrice", "stop_price"),
];

/// `store.update_order`: a status, the broker's own order id, or an error on
/// a ticket that already exists. Only the named fields are touched.
pub fn update_order(conn: &Connection, order_id: &str, patch: &Value, now: &str) -> Result<()> {
    let p = match patch.as_object() { Some(p) => p, None => return Ok(()) };
    let mut sets: Vec<String> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    for (k, col) in ORDER_TEXT {
        if p.contains_key(k) {
            sets.push(format!("{} = ?", col));
            vals.push(Box::new(field_s(patch, k)));
        }
    }
    for (k, col) in ORDER_NUM {
        if p.contains_key(k) {
            sets.push(format!("{} = ?", col));
            vals.push(Box::new(opt_num(p.get(k))));
        }
    }
    if sets.is_empty() {
        return Ok(());
    }
    sets.push("updated_at = ?".into());
    vals.push(Box::new(now.to_string()));
    vals.push(Box::new(order_id.to_string()));
    let sql = format!("UPDATE orders SET {} WHERE id = ?", sets.join(", "));
    let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    conn.execute(&sql, refs.as_slice())?;
    Ok(())
}

/// `store.list_orders`: newest first.
pub fn list_orders(conn: &Connection, limit: i64) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM orders ORDER BY created_at DESC, rowid DESC LIMIT ?")?;
    let mut rows = stmt.query([limit])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(order_from_row(r)?);
    }
    Ok(out)
}

pub fn get_order(conn: &Connection, order_id: &str) -> Result<Option<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM orders WHERE id = ?")?;
    let mut rows = stmt.query([order_id])?;
    match rows.next()? {
        Some(r) => Ok(Some(order_from_row(r)?)),
        None => Ok(None),
    }
}

/// `store.mark_order_fill_booked`: record that this order's fill has been
/// written as a local activity, so a later status poll does not book it twice.
///
/// The booked quantity only ever grows; a smaller value never lowers it.
///
/// The answer is whether *this* statement changed a row. Python returns
/// `conn.total_changes > 0`, which counts every change the pooled connection
/// has ever made, so it reads true even for an order id that does not exist.
/// Nothing reads the result there, but `stamp_canonical_id` has the same
/// shape and its result is acted on -- see the note on that function.
pub fn mark_order_fill_booked(conn: &Connection, order_id: &str, qty: f64, now: &str) -> Result<bool> {
    let n = conn.execute(
        "UPDATE orders SET fill_booked_qty = ?, updated_at = ? WHERE id = ? AND (fill_booked_qty IS NULL OR fill_booked_qty < ?)",
        rusqlite::params![qty, now, order_id, qty],
    )?;
    Ok(n > 0)
}

// --------------------------------------------------------------------------
// brackets
// --------------------------------------------------------------------------

/// `store._bracket_from_row`.
pub fn bracket_from_row(r: &Row) -> Result<Value> {
    let tif = { let t = text(r, "tif")?; if t.is_empty() { "DAY".to_string() } else { t } };
    let unit = { let t = text(r, "sl_trail_unit")?; if t.is_empty() { "pct".to_string() } else { t } };
    Ok(json!({
        "id": text(r, "id")?,
        "orderId": text(r, "order_id")?,
        "createdAt": text(r, "created_at")?,
        "accountId": text(r, "account_id")?,
        "securityId": text(r, "security_id")?,
        "symbol": text(r, "symbol")?,
        "currency": text(r, "currency")?,
        "quantity": real(r, "quantity")?,
        "tif": tif,
        "slKind": text(r, "sl_kind")?,
        "slPrice": real(r, "sl_price")?,
        "slTrail": real(r, "sl_trail")?,
        "slTrailUnit": unit,
        "slOrderId": text(r, "sl_order_id")?,
        "slNative": r.get::<_, Option<i64>>("sl_native")?.unwrap_or(0) != 0,
        "slMode": text(r, "sl_mode")?,
        "highWater": real(r, "high_water")?,
        "tpPrice": real(r, "tp_price")?,
        "tpOrderId": text(r, "tp_order_id")?,
        "status": text(r, "status")?,
        "outcome": text(r, "outcome")?,
        "error": text(r, "error")?,
        "attempts": r.get::<_, Option<i64>>("attempts")?.unwrap_or(0),
        "movedAt": text(r, "moved_at")?,
        "armedAt": text(r, "armed_at")?,
        "seenHeld": r.get::<_, Option<i64>>("seen_held")?.unwrap_or(0) != 0,
        "missedAt": text(r, "missed_at")?,
        "updatedAt": text(r, "updated_at")?,
    }))
}

/// `store.BRACKET_TEXT`.
const BRACKET_TEXT: [(&str, &str); 14] = [
    ("symbol", "symbol"), ("currency", "currency"), ("tif", "tif"), ("slKind", "sl_kind"),
    ("slTrailUnit", "sl_trail_unit"), ("slOrderId", "sl_order_id"), ("tpOrderId", "tp_order_id"),
    ("status", "status"), ("outcome", "outcome"), ("error", "error"), ("movedAt", "moved_at"),
    ("armedAt", "armed_at"), ("slMode", "sl_mode"), ("missedAt", "missed_at"),
];
/// `store.BRACKET_NUM`.
const BRACKET_NUM: [(&str, &str); 8] = [
    ("quantity", "quantity"), ("slPrice", "sl_price"), ("slTrail", "sl_trail"),
    ("highWater", "high_water"), ("tpPrice", "tp_price"), ("attempts", "attempts"),
    ("slNative", "sl_native"), ("seenHeld", "seen_held"),
];

pub fn insert_bracket(conn: &Connection, b: &Value, now: &str) -> Result<()> {
    let created = { let c = field_s(b, "createdAt"); if c.is_empty() { now.to_string() } else { c } };
    let tif = { let t = field_s(b, "tif"); if t.is_empty() { "DAY".to_string() } else { t } };
    let unit = { let t = field_s(b, "slTrailUnit"); if t.is_empty() { "pct".to_string() } else { t } };
    let status = { let t = field_s(b, "status"); if t.is_empty() { "waiting".to_string() } else { t } };
    let attempts = num(get(b, "attempts"), 0.0) as i64;
    conn.execute(
        "INSERT INTO brackets (id, order_id, created_at, account_id, security_id, symbol, currency, quantity, tif, sl_kind, sl_price, sl_trail, \
         sl_trail_unit, sl_order_id, sl_native, sl_mode, high_water, tp_price, tp_order_id, status, outcome, error, attempts, moved_at, armed_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            field_s(b, "id"), field_s(b, "orderId"), created, field_s(b, "accountId"), field_s(b, "securityId"),
            field_s(b, "symbol"), field_s(b, "currency"), opt_num(get(b, "quantity")), tif,
            field_s(b, "slKind"), opt_num(get(b, "slPrice")), opt_num(get(b, "slTrail")), unit,
            field_s(b, "slOrderId"), if get(b, "slNative").map(truthy).unwrap_or(false) { 1 } else { 0 },
            field_s(b, "slMode"), opt_num(get(b, "highWater")), opt_num(get(b, "tpPrice")),
            field_s(b, "tpOrderId"), status, field_s(b, "outcome"), field_s(b, "error"), attempts,
            field_s(b, "movedAt"), field_s(b, "armedAt"), now,
        ],
    )?;
    Ok(())
}

pub fn update_bracket(conn: &Connection, bracket_id: &str, patch: &Value, now: &str) -> Result<()> {
    let p = match patch.as_object() { Some(p) => p, None => return Ok(()) };
    let mut sets: Vec<String> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    for (k, col) in BRACKET_TEXT {
        if p.contains_key(k) {
            sets.push(format!("{} = ?", col));
            vals.push(Box::new(field_s(patch, k)));
        }
    }
    for (k, col) in BRACKET_NUM {
        if let Some(v) = p.get(k) {
            sets.push(format!("{} = ?", col));
            if v.is_null() {
                vals.push(Box::new(None::<f64>));
            } else if k == "slNative" || k == "seenHeld" {
                vals.push(Box::new(if truthy(v) { 1i64 } else { 0i64 }));
            } else if k == "attempts" {
                vals.push(Box::new(num(Some(v), 0.0) as i64));
            } else {
                vals.push(Box::new(opt_num(Some(v))));
            }
        }
    }
    if sets.is_empty() {
        return Ok(());
    }
    sets.push("updated_at = ?".into());
    vals.push(Box::new(now.to_string()));
    vals.push(Box::new(bracket_id.to_string()));
    let sql = format!("UPDATE brackets SET {} WHERE id = ?", sets.join(", "));
    let refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    conn.execute(&sql, refs.as_slice())?;
    Ok(())
}

pub fn list_brackets(conn: &Connection, statuses: &[String]) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    if statuses.is_empty() {
        let mut stmt = conn.prepare("SELECT * FROM brackets ORDER BY created_at")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            out.push(bracket_from_row(r)?);
        }
        return Ok(out);
    }
    let marks = statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("SELECT * FROM brackets WHERE status IN ({}) ORDER BY created_at", marks);
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = statuses.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let mut rows = stmt.query(params.as_slice())?;
    while let Some(r) = rows.next()? {
        out.push(bracket_from_row(r)?);
    }
    Ok(out)
}

pub fn get_bracket(conn: &Connection, bracket_id: &str) -> Result<Option<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM brackets WHERE id = ?")?;
    let mut rows = stmt.query([bracket_id])?;
    match rows.next()? { Some(r) => Ok(Some(bracket_from_row(r)?)), None => Ok(None) }
}

pub fn bracket_for_order(conn: &Connection, order_id: &str) -> Result<Option<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM brackets WHERE order_id = ? ORDER BY created_at DESC LIMIT 1")?;
    let mut rows = stmt.query([order_id])?;
    match rows.next()? { Some(r) => Ok(Some(bracket_from_row(r)?)), None => Ok(None) }
}

/// `store.symbol_for_security`: the symbol the book uses for a security, from
/// its activity rows.
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
    if open_statuses.is_empty() {
        return Ok(0);
    }
    let marks = open_statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("SELECT COUNT(*) FROM orders WHERE status IN ({})", marks);
    let params: Vec<&dyn rusqlite::ToSql> = open_statuses.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    conn.query_row(&sql, params.as_slice(), |r| r.get(0))
}

/// Kept for callers holding a patch as a map.
pub fn patch_of(m: Map<String, Value>) -> Value {
    Value::Object(m)
}
