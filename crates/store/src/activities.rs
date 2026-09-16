//! Activity rows: the shape the model reads, and the shape the table holds.
//!
//! A Wealthsimple row keeps the broker's own id as its canonical id, so the
//! same fill pulled twice is one row. A typed or imported row never gets one
//! fabricated for it.

use rusqlite::{Connection, Result, Row};
use serde_json::{json, Map, Value};

use bagholder_model::value::{field_s, get, num, s as vs};

pub const INVENTED_ACCOUNTS: [&str; 7] = ["", "manual", "legacy", "statement", "canonical", "cad", "usd"];

/// Every column of `activities`, in the order the insert names them.
pub const COLUMNS: [&str; 27] = [
    "id", "canonical_id", "occurred_at", "transaction_date", "settlement_date", "account_id", "book_id",
    "fifo_id", "account_type", "activity_type", "activity_sub_type", "description", "direction", "symbol",
    "name", "currency", "quantity", "unit_price", "commission", "net_cash_amount", "category", "balance",
    "source", "raw_type", "aft_type", "counter_symbol", "security_id",
];

const INSERT_SQL: &str = "INSERT INTO activities (
        id, canonical_id, occurred_at, transaction_date, settlement_date,
        account_id, book_id, fifo_id, account_type, activity_type,
        activity_sub_type, description, direction, symbol, name, currency,
        quantity, unit_price, commission, net_cash_amount, category, balance,
        source, raw_type, aft_type, counter_symbol, security_id
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

/// `store.looks_like_homemade_id`: an id Bagholder made, not the broker.
pub fn looks_like_homemade_id(aid: &str) -> bool {
    let s = aid.trim();
    s.is_empty() || s.contains('|') || s.to_lowercase().starts_with("manual")
}

/// `store.is_real_account`: an account Wealthsimple actually has, as opposed
/// to the placeholders an import invents.
pub fn is_real_account(account_id: &str) -> bool {
    let s = account_id.trim();
    if s.is_empty() || s.starts_with('~') {
        return false;
    }
    !INVENTED_ACCOUNTS.contains(&s.to_lowercase().as_str())
}

/// `store._round_qty`: eight decimals, which is what a crypto quantity needs
/// and what the match keys compare on.
pub fn round_qty(v: Option<&Value>) -> f64 {
    let n = num(v, 0.0);
    (n * 1e8).round() / 1e8
}

/// Either spelling of a field, camelCase first, as the Python readers accept.
fn either(act: &Value, camel: &str, snake: &str) -> String {
    let v = field_s(act, camel);
    if v.is_empty() { field_s(act, snake) } else { v }
}

fn either_val<'a>(act: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    get(act, camel).or_else(|| get(act, snake))
}

/// `store.trade_side`.
pub fn trade_side(act: &Value) -> String {
    bagholder_model::fifo::trade_side(act)
}

fn key_date(act: &Value) -> String {
    let d: String = either(act, "transactionDate", "transaction_date").chars().take(10).collect();
    if !d.is_empty() {
        return d;
    }
    let occurred = either(act, "occurredAt", "occurred_at");
    occurred.split('T').next().unwrap_or("").chars().take(10).collect()
}

fn key_account(act: &Value, include_account: bool) -> String {
    if !include_account {
        return String::new();
    }
    let aid = either(act, "accountId", "account_id");
    if is_real_account(&aid) { aid } else { String::new() }
}

/// The price and cash as the match keys read them: under the camelCase name
/// only.
///
/// `store.field_match_key` spells this
/// `act.get("unitPrice") if "unitPrice" in act or "unit_price" in act else act.get("unit_price")`,
/// and both branches of that read `unitPrice`, so a row in the snake_case
/// spelling matches on 0.0 rather than on its price. `_insert_params` tests
/// only `"unitPrice" in act` and does fall back, so a stored row is unaffected.
/// Reproduced rather than corrected: changing it would change which rows the
/// sync considers the same fill.
fn key_price(act: &Value, camel: &str) -> f64 {
    round_qty(get(act, camel))
}

/// `store.field_match_key`: what makes two rows the same fill when neither
/// carries the broker's id.
pub fn field_match_key(act: &Value, include_account: bool) -> (String, String, String, f64, f64, f64) {
    (
        key_date(act),
        key_account(act, include_account),
        field_s(act, "symbol").trim().to_uppercase(),
        round_qty(get(act, "quantity")),
        key_price(act, "unitPrice"),
        key_price(act, "netCashAmount"),
    )
}

/// `store.link_match_key`: the same, ordered for linking an imported row to a
/// broker one.
pub fn link_match_key(act: &Value, include_account: bool) -> (String, String, f64, f64, String, String) {
    (
        field_s(act, "symbol").trim().to_uppercase(),
        trade_side(act),
        round_qty(get(act, "quantity")),
        key_price(act, "unitPrice"),
        key_date(act),
        key_account(act, include_account),
    )
}

/// `store._canonical_from_row`: the broker's id, never one Bagholder made.
pub fn canonical_from_row(act: &Value, source: &str) -> Option<String> {
    if source != "wealthsimple" {
        return None;
    }
    let cid = either(act, "canonicalId", "canonical_id").trim().to_string();
    if !cid.is_empty() && !looks_like_homemade_id(&cid) {
        return Some(cid);
    }
    let old_id = field_s(act, "id").trim().to_string();
    if !old_id.is_empty() && !looks_like_homemade_id(&old_id) {
        return Some(old_id);
    }
    None
}

fn text(row: &Row, idx: usize) -> rusqlite::Result<String> {
    Ok(row.get::<_, Option<String>>(idx)?.unwrap_or_default())
}

fn real(row: &Row, idx: usize) -> rusqlite::Result<Value> {
    Ok(match row.get::<_, Option<f64>>(idx)? {
        Some(v) => json!(v),
        None => Value::Null,
    })
}

/// `store._row_to_activity`: the row as the model wants it, with the fallbacks
/// the Python reader applies -- a missing settlement date is the transaction
/// date, a missing book or fifo id the account's.
pub fn row_to_activity(row: &Row) -> rusqlite::Result<Value> {
    let canonical = row.get::<_, Option<String>>(1)?.filter(|s| !s.is_empty());
    let transaction_date = text(row, 3)?;
    let account_id = text(row, 5)?;
    let settlement = { let s = text(row, 4)?; if s.is_empty() { transaction_date.clone() } else { s } };
    let book_id = { let s = text(row, 6)?; if s.is_empty() { account_id.clone() } else { s } };
    let fifo_id = { let s = text(row, 7)?; if s.is_empty() { account_id.clone() } else { s } };
    let security_id = row.get::<_, Option<String>>(26)?.filter(|s| !s.is_empty());

    Ok(json!({
        "id": text(row, 0)?,
        "canonicalId": canonical,
        "occurredAt": text(row, 2)?,
        "transactionDate": transaction_date,
        "settlementDate": settlement,
        "accountId": account_id,
        "bookId": book_id,
        "fifoId": fifo_id,
        "accountType": text(row, 8)?,
        "activityType": text(row, 9)?,
        "activitySubType": text(row, 10)?,
        "description": text(row, 11)?,
        "direction": text(row, 12)?,
        "symbol": text(row, 13)?,
        "name": text(row, 14)?,
        "currency": text(row, 15)?,
        "quantity": real(row, 16)?,
        "unitPrice": real(row, 17)?,
        "commission": real(row, 18)?,
        "netCashAmount": real(row, 19)?,
        "category": text(row, 20)?,
        "balance": real(row, 21)?,
        "source": text(row, 22)?,
        "rawType": text(row, 23)?,
        "aftType": text(row, 24)?,
        "counterSymbol": text(row, 25)?,
        "securityId": security_id,
    }))
}

const SELECT_ALL: &str = "SELECT id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, account_type, activity_type, activity_sub_type, description, direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, balance, source, raw_type, aft_type, counter_symbol, security_id FROM activities";

/// `store._all_activities`: every row, oldest first.
pub fn all_activities(conn: &Connection) -> Result<Vec<Value>> {
    let sql = format!("{SELECT_ALL} ORDER BY COALESCE(occurred_at, transaction_date) ASC, id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_activity)?;
    rows.collect()
}

pub fn activity_by_id(conn: &Connection, id: &str) -> Result<Option<Value>> {
    let sql = format!("{SELECT_ALL} WHERE id = ?");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(r) => Ok(Some(row_to_activity(r)?)),
        None => Ok(None),
    }
}

/// A number stored as NULL rather than zero, as Python's `_num(v, None)`.
fn opt_real(act: &Value, camel: &str, snake: &str) -> Value {
    match either_val(act, camel, snake) {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) if s.is_empty() => Value::Null,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { Value::Null } else { json!(n) }
        }
    }
}

fn real_or_zero(act: &Value, camel: &str, snake: &str) -> f64 {
    match either_val(act, camel, snake) {
        None | Some(Value::Null) => 0.0,
        Some(Value::String(s)) if s.is_empty() => 0.0,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { 0.0 } else { n }
        }
    }
}

/// `store._insert_params`.
fn insert_params(act: &Value, assigned_id: &str, canonical_id: Option<&str>) -> Vec<Value> {
    let mut occurred = either(act, "occurredAt", "occurred_at").trim().to_string();
    let mut date = either(act, "transactionDate", "transaction_date").trim().to_string();
    if date.is_empty() && !occurred.is_empty() {
        date = occurred.split('T').next().unwrap_or("").chars().take(10).collect();
    }
    if !occurred.is_empty() && !occurred.contains('T') {
        // a date-only source (typed in, or a CSV) stays date-only
        occurred = occurred.chars().take(10).collect();
    }
    let settle = {
        let s = either(act, "settlementDate", "settlement_date").trim().to_string();
        if s.is_empty() { date.clone() } else { s }
    };
    let account_id = either(act, "accountId", "account_id");
    let or_account = |camel: &str, snake: &str| {
        let v = either(act, camel, snake);
        if v.is_empty() { account_id.clone() } else { v }
    };
    let security_id = either(act, "securityId", "security_id").trim().to_string();

    vec![
        json!(assigned_id),
        canonical_id.map(|c| json!(c)).unwrap_or(Value::Null),
        json!(occurred),
        json!(date),
        json!(settle),
        json!(account_id),
        json!(or_account("bookId", "book_id")),
        json!(or_account("fifoId", "fifo_id")),
        json!(either(act, "accountType", "account_type")),
        json!(either(act, "activityType", "activity_type")),
        json!(either(act, "activitySubType", "activity_sub_type")),
        json!(field_s(act, "description")),
        json!(field_s(act, "direction")),
        json!(field_s(act, "symbol")),
        json!(field_s(act, "name")),
        json!(field_s(act, "currency")),
        json!(real_or_zero(act, "quantity", "quantity")),
        json!(real_or_zero(act, "unitPrice", "unit_price")),
        json!(real_or_zero(act, "commission", "commission")),
        json!(real_or_zero(act, "netCashAmount", "net_cash_amount")),
        json!(field_s(act, "category")),
        opt_real(act, "balance", "balance"),
        json!(field_s(act, "source")),
        json!(either(act, "rawType", "raw_type")),
        json!(either(act, "aftType", "aft_type")),
        json!(either(act, "counterSymbol", "counter_symbol")),
        if security_id.is_empty() { Value::Null } else { json!(security_id) },
    ]
}

fn bind(params: &[Value]) -> Vec<Box<dyn rusqlite::ToSql>> {
    params
        .iter()
        .map(|v| -> Box<dyn rusqlite::ToSql> {
            match v {
                Value::Null => Box::new(None::<String>),
                Value::String(s) => Box::new(s.clone()),
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() { Box::new(i) } else { Box::new(n.as_f64().unwrap_or(0.0)) }
                }
                Value::Bool(b) => Box::new(*b as i64),
                other => Box::new(other.to_string()),
            }
        })
        .collect()
}

/// `store.insert_activity`: one row. The caller decides the canonical id, and
/// none is ever fabricated.
pub fn insert_activity(
    conn: &Connection,
    act: &Value,
    canonical_id: Option<&str>,
    assigned_id: Option<&str>,
    new_id: &dyn Fn() -> String,
) -> Result<Value> {
    let source = { let s = field_s(act, "source"); if s.is_empty() { "wealthsimple".to_string() } else { s } };
    let mut canonical: Option<String> = canonical_id.map(|c| c.to_string());
    if canonical.is_none() && source == "wealthsimple" {
        canonical = canonical_from_row(act, &source);
    }
    if source != "wealthsimple" {
        canonical = None;
    }
    if canonical.as_deref() == Some("") {
        canonical = None;
    }
    let mut aid = assigned_id.map(|s| s.to_string()).unwrap_or_else(|| field_s(act, "id").trim().to_string());
    if aid.is_empty() || looks_like_homemade_id(&aid) {
        aid = new_id();
    }
    let params = insert_params(act, &aid, canonical.as_deref());
    let boxed = bind(&params);
    let refs: Vec<&dyn rusqlite::ToSql> = boxed.iter().map(|b| b.as_ref()).collect();
    conn.execute(INSERT_SQL, refs.as_slice())?;
    Ok(activity_by_id(conn, &aid)?.unwrap_or(Value::Null))
}

/// `store.insert_local`: a typed-in or imported row. It gets a Bagholder id
/// and never a fabricated canonical id.
pub fn insert_local(conn: &Connection, act: &Value, new_id: &dyn Fn() -> String) -> Result<Value> {
    let mut payload: Map<String, Value> = match act {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    let mut source = vs(payload.get("source"));
    if source.is_empty() || source == "wealthsimple" {
        source = "manual".into();
    }
    payload.insert("source".into(), json!(source));
    payload.remove("canonicalId");
    payload.remove("canonical_id");
    insert_activity(conn, &Value::Object(payload), None, None, new_id)
}

/// `store.activity_count`.
pub fn activity_count(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0))
}

/// `store.canonical_ids`: every broker id the store already holds.
pub fn canonical_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT canonical_id FROM activities WHERE canonical_id IS NOT NULL AND canonical_id != ''")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}
