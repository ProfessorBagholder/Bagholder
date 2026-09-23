//! Orders and brackets, read and written as what they are.

use rusqlite::{Connection, Result, Row};
use serde_json::Value;

use super::types::*;

fn text(r: &Row, name: &str) -> Result<String> {
    Ok(r.get::<_, Option<String>>(name)?.unwrap_or_default())
}

/// A JSON column, read as `T`; nothing when it is empty or is not a `T`.
fn js<T: serde::de::DeserializeOwned>(r: &Row, name: &str) -> Result<Option<T>> {
    let raw: Option<String> = r.get(name)?;
    Ok(raw.filter(|s| !s.is_empty()).and_then(|s| serde_json::from_str(&s).ok()))
}

fn order_of(r: &Row) -> Result<Order> {
    let source = Source::parse(&text(r, "source")?);
    let role = Role::parse(&text(r, "role")?);
    Ok(Order {
        id: text(r, "id")?,
        created_at: text(r, "created_at")?,
        account_id: text(r, "account_id")?,
        account: text(r, "account")?,
        security_id: text(r, "security_id")?,
        symbol: text(r, "symbol")?,
        currency: text(r, "currency")?,
        side: Side::parse(&text(r, "side")?),
        kind: OrderType::parse(&text(r, "type")?),
        quantity: r.get("quantity")?,
        limit_price: r.get("limit_price")?,
        stop_price: r.get("stop_price")?,
        tif: text(r, "tif")?,
        stop_loss: js(r, "stop_loss")?,
        take_profit: js(r, "take_profit")?,
        status: OrderStatus::parse(&text(r, "status")?),
        ws_order_id: text(r, "ws_order_id")?,
        error: text(r, "error")?,
        request: js::<Value>(r, "request")?.unwrap_or(Value::Null),
        updated_at: text(r, "updated_at")?,
        source: if source.is_set() { source } else { Source::Bagholder },
        ws_status: text(r, "ws_status")?,
        filled_qty: r.get("filled_qty")?,
        avg_fill: r.get("avg_fill")?,
        submitted_at: text(r, "submitted_at")?,
        expires_at: text(r, "expires_at")?,
        parent_id: text(r, "parent_id")?,
        role: if role.is_set() { role } else { Role::Entry },
        fill_booked_qty: r.get("fill_booked_qty")?,
    })
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

/// A new ticket, written before anything is sent.
pub fn insert_order(conn: &Connection, o: &Order, now: &str) -> Result<()> {
    let created = if o.created_at.is_empty() { now } else { o.created_at.as_str() };
    // the request is stored with its keys sorted, so the same ticket is the same text
    // whichever order it was built in
    let request = Some(&o.request).filter(|v| truthy(v)).map(crate::tables::json_text_sorted);
    let as_text = |v: Value| crate::tables::json_text(&v);
    let stop_loss = o.stop_loss.as_ref().map(|s| as_text(serde_json::to_value(s).unwrap_or(Value::Null)));
    let take_profit = o.take_profit.as_ref().map(|t| as_text(serde_json::to_value(t).unwrap_or(Value::Null)));
    conn.execute(
        "INSERT INTO orders (id, created_at, account_id, account, security_id, symbol, currency, side, type, quantity, \
         limit_price, stop_price, tif, stop_loss, take_profit, status, ws_order_id, error, request, updated_at, \
         source, ws_status, filled_qty, avg_fill, submitted_at, expires_at, parent_id, role) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            o.id, created, o.account_id, o.account, o.security_id, o.symbol, o.currency,
            o.side, o.kind, o.quantity.unwrap_or(0.0), o.limit_price, o.stop_price, o.tif,
            stop_loss, take_profit, o.status, o.ws_order_id, o.error, request, now,
            if o.source.is_set() { o.source } else { Source::Bagholder }, o.ws_status, o.filled_qty, o.avg_fill,
            o.submitted_at, o.expires_at, o.parent_id, if o.role.is_set() { o.role } else { Role::Entry },
        ],
    )?;
    Ok(())
}

/// The SET clause of a patch: `column = ?` for each field it carries, and the values.
struct Sets {
    cols: Vec<&'static str>,
    vals: Vec<Box<dyn rusqlite::ToSql>>,
}
impl Sets {
    fn new() -> Sets { Sets { cols: Vec::new(), vals: Vec::new() } }
    fn put<T: rusqlite::ToSql + Clone + 'static>(&mut self, col: &'static str, v: &Option<T>) {
        if let Some(v) = v {
            self.cols.push(col);
            self.vals.push(Box::new(v.clone()));
        }
    }
    fn run(mut self, conn: &Connection, table: &str, id: &str, now: &str) -> Result<()> {
        if self.cols.is_empty() {
            return Ok(());
        }
        let mut sql: Vec<String> = self.cols.iter().map(|c| format!("{} = ?", c)).collect();
        sql.push("updated_at = ?".into());
        self.vals.push(Box::new(now.to_string()));
        self.vals.push(Box::new(id.to_string()));
        let refs: Vec<&dyn rusqlite::ToSql> = self.vals.iter().map(|b| b.as_ref()).collect();
        conn.execute(&format!("UPDATE {} SET {} WHERE id = ?", table, sql.join(", ")), refs.as_slice())?;
        Ok(())
    }
}

/// A status, the broker's own order id, or an error on a ticket that already exists.
/// Only the fields the patch carries are touched.
pub fn update_order(conn: &Connection, order_id: &str, p: &OrderPatch, now: &str) -> Result<()> {
    crate::atomically(conn, || {
        let mut s = Sets::new();
        s.put("status", &p.status);
        s.put("ws_order_id", &p.ws_order_id);
        s.put("error", &p.error);
        s.put("ws_status", &p.ws_status);
        s.put("submitted_at", &p.submitted_at);
        s.put("expires_at", &p.expires_at);
        s.put("tif", &p.tif);
        s.put("currency", &p.currency);
        s.put("symbol", &p.symbol);
        s.put("filled_qty", &p.filled_qty);
        s.put("avg_fill", &p.avg_fill);
        s.put("quantity", &p.quantity);
        s.put("limit_price", &p.limit_price);
        s.put("stop_price", &p.stop_price);
        s.run(conn, "orders", order_id, now)
    })
}

/// Newest first.
pub fn list_orders(conn: &Connection, limit: i64) -> Result<Vec<Order>> {
    let mut stmt = conn.prepare("SELECT * FROM orders ORDER BY created_at DESC, rowid DESC LIMIT ?")?;
    let rows = stmt.query_map([limit], order_of)?;
    rows.collect()
}

pub fn get_order(conn: &Connection, order_id: &str) -> Result<Option<Order>> {
    let mut stmt = conn.prepare("SELECT * FROM orders WHERE id = ?")?;
    let mut rows = stmt.query_map([order_id], order_of)?;
    rows.next().transpose()
}

fn bracket_of(r: &Row) -> Result<Bracket> {
    let tif = text(r, "tif")?;
    let unit = TrailUnit::parse(&text(r, "sl_trail_unit")?);
    Ok(Bracket {
        id: text(r, "id")?,
        order_id: text(r, "order_id")?,
        created_at: text(r, "created_at")?,
        account_id: text(r, "account_id")?,
        security_id: text(r, "security_id")?,
        symbol: text(r, "symbol")?,
        currency: text(r, "currency")?,
        quantity: r.get("quantity")?,
        tif: if tif.is_empty() { "DAY".into() } else { tif },
        sl_kind: SlKind::parse(&text(r, "sl_kind")?),
        sl_price: r.get("sl_price")?,
        sl_trail: r.get("sl_trail")?,
        sl_trail_unit: if unit.is_set() { unit } else { TrailUnit::Pct },
        sl_order_id: text(r, "sl_order_id")?,
        sl_native: r.get::<_, Option<i64>>("sl_native")?.unwrap_or(0) != 0,
        sl_mode: SlMode::parse(&text(r, "sl_mode")?),
        high_water: r.get("high_water")?,
        tp_price: r.get("tp_price")?,
        tp_order_id: text(r, "tp_order_id")?,
        status: BracketStatus::parse(&text(r, "status")?),
        outcome: text(r, "outcome")?,
        error: text(r, "error")?,
        attempts: r.get::<_, Option<i64>>("attempts")?.unwrap_or(0),
        moved_at: text(r, "moved_at")?,
        armed_at: text(r, "armed_at")?,
        seen_held: r.get::<_, Option<i64>>("seen_held")?.unwrap_or(0) != 0,
        missed_at: text(r, "missed_at")?,
        updated_at: text(r, "updated_at")?,
    })
}

pub fn insert_bracket(conn: &Connection, b: &Bracket, now: &str) -> Result<()> {
    let created = if b.created_at.is_empty() { now } else { b.created_at.as_str() };
    conn.execute(
        "INSERT INTO brackets (id, order_id, created_at, account_id, security_id, symbol, currency, quantity, tif, sl_kind, sl_price, sl_trail, \
         sl_trail_unit, sl_order_id, sl_native, sl_mode, high_water, tp_price, tp_order_id, status, outcome, error, attempts, moved_at, armed_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            b.id, b.order_id, created, b.account_id, b.security_id, b.symbol, b.currency, b.quantity,
            if b.tif.is_empty() { "DAY" } else { b.tif.as_str() },
            b.sl_kind, b.sl_price, b.sl_trail, if b.sl_trail_unit.is_set() { b.sl_trail_unit } else { TrailUnit::Pct },
            b.sl_order_id, b.sl_native as i64, b.sl_mode, b.high_water, b.tp_price, b.tp_order_id,
            if b.status.is_set() { b.status } else { BracketStatus::Waiting }, b.outcome, b.error, b.attempts,
            b.moved_at, b.armed_at, now,
        ],
    )?;
    Ok(())
}

pub fn update_bracket(conn: &Connection, bracket_id: &str, p: &BracketPatch, now: &str) -> Result<()> {
    crate::atomically(conn, || {
        let mut s = Sets::new();
        s.put("symbol", &p.symbol);
        s.put("currency", &p.currency);
        s.put("tif", &p.tif);
        s.put("sl_kind", &p.sl_kind);
        s.put("sl_trail_unit", &p.sl_trail_unit);
        s.put("sl_order_id", &p.sl_order_id);
        s.put("tp_order_id", &p.tp_order_id);
        s.put("status", &p.status);
        s.put("outcome", &p.outcome);
        s.put("error", &p.error);
        s.put("moved_at", &p.moved_at);
        s.put("armed_at", &p.armed_at);
        s.put("sl_mode", &p.sl_mode);
        s.put("missed_at", &p.missed_at);
        s.put("quantity", &p.quantity);
        s.put("sl_price", &p.sl_price);
        s.put("sl_trail", &p.sl_trail);
        s.put("high_water", &p.high_water);
        s.put("tp_price", &p.tp_price);
        s.put("attempts", &p.attempts);
        s.put("sl_native", &p.sl_native.map(|b| b as i64));
        s.put("seen_held", &p.seen_held.map(|b| b as i64));
        s.run(conn, "brackets", bracket_id, now)
    })
}

/// Every bracket, oldest first; or only those standing at one of `statuses`.
pub fn list_brackets(conn: &Connection, statuses: &[BracketStatus]) -> Result<Vec<Bracket>> {
    if statuses.is_empty() {
        let mut stmt = conn.prepare("SELECT * FROM brackets ORDER BY created_at")?;
        let rows = stmt.query_map([], bracket_of)?;
        return rows.collect();
    }
    let marks = statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let mut stmt = conn.prepare(&format!("SELECT * FROM brackets WHERE status IN ({}) ORDER BY created_at", marks))?;
    let params: Vec<&dyn rusqlite::ToSql> = statuses.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(params.as_slice(), bracket_of)?;
    rows.collect()
}

pub fn get_bracket(conn: &Connection, bracket_id: &str) -> Result<Option<Bracket>> {
    let mut stmt = conn.prepare("SELECT * FROM brackets WHERE id = ?")?;
    let mut rows = stmt.query_map([bracket_id], bracket_of)?;
    rows.next().transpose()
}

/// The bracket an entry order has, the newest if it was given more than one.
pub fn bracket_for_order(conn: &Connection, order_id: &str) -> Result<Option<Bracket>> {
    let mut stmt = conn.prepare("SELECT * FROM brackets WHERE order_id = ? ORDER BY created_at DESC LIMIT 1")?;
    let mut rows = stmt.query_map([order_id], bracket_of)?;
    rows.next().transpose()
}

/// The open orders the header counts.
pub fn open_orders_count(conn: &Connection, open: &[OrderStatus]) -> Result<i64> {
    if open.is_empty() {
        return Ok(0);
    }
    let marks = open.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let params: Vec<&dyn rusqlite::ToSql> = open.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    conn.query_row(&format!("SELECT COUNT(*) FROM orders WHERE status IN ({})", marks), params.as_slice(), |r| r.get(0))
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
