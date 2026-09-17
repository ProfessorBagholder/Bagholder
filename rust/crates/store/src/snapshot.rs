//! `snapshot`: everything the model is built from, in one read.

use rusqlite::{Connection, Result, Row};
use serde_json::{json, Map, Value};

use crate::activities::all_activities;
use crate::tables::{clean_trade_groups, clean_trade_notes, get_meta};

pub const TILES_META: &str = "market_tiles";

fn text(row: &Row, name: &str) -> rusqlite::Result<String> {
    Ok(row.get::<_, Option<String>>(name)?.unwrap_or_default())
}

fn opt_text(row: &Row, name: &str) -> rusqlite::Result<Value> {
    Ok(match row.get::<_, Option<String>>(name)? {
        Some(s) if !s.is_empty() => json!(s),
        _ => Value::Null,
    })
}

fn real(row: &Row, name: &str) -> rusqlite::Result<Value> {
    Ok(match row.get::<_, Option<f64>>(name)? {
        Some(v) => json!(v),
        None => Value::Null,
    })
}

/// `_security_from_row`.
fn security_from_row(r: &Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": text(r, "id")?,
        "symbol": text(r, "symbol")?,
        "name": text(r, "name")?,
        "primaryExchange": text(r, "primary_exchange")?,
        "primaryMic": text(r, "primary_mic")?,
        "currency": text(r, "currency")?,
        "underlyingId": opt_text(r, "underlying_id")?,
    }))
}

/// `_exposure_from_row`: the two weight maps are stored as JSON text, and
/// anything unreadable is simply no exposure rather than an error.
fn exposure_from_row(r: &Row) -> rusqlite::Result<Value> {
    let js = |v: Option<String>| -> Value {
        match v {
            Some(s) if !s.is_empty() => serde_json::from_str(&s).unwrap_or_else(|_| json!({})),
            _ => json!({}),
        }
    };
    Ok(json!({
        "sectors": js(r.get::<_, Option<String>>("sectors")?),
        "countries": js(r.get::<_, Option<String>>("countries")?),
        "coverage": r.get::<_, Option<f64>>("coverage")?.unwrap_or(0.0),
        "source": text(r, "source")?,
        "asOf": text(r, "as_of")?,
        "industry": text(r, "industry")?,
        "error": text(r, "error")?,
        "fetchedAt": text(r, "fetched_at")?,
    }))
}

/// `_watch_from_row`.
fn watch_from_row(r: &Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "symbol": text(r, "symbol")?,
        "exchange": text(r, "exchange")?,
        "name": text(r, "name")?,
        "currency": text(r, "currency")?,
        "securityId": text(r, "security_id")?,
        "addedAt": text(r, "added_at")?,
    }))
}

/// `_news_from_row`: an item with no kind is a story, which is what a
/// row stored before releases were told apart is.
pub fn news_from_row(r: &Row) -> rusqlite::Result<Value> {
    let kind = { let k = text(r, "kind")?; if k.is_empty() { "story".to_string() } else { k } };
    Ok(json!({
        "id": text(r, "id")?,
        "symbol": text(r, "symbol")?,
        "exchange": text(r, "exchange")?,
        "source": text(r, "source")?,
        "headline": text(r, "headline")?,
        "wire": text(r, "wire")?,
        "url": text(r, "url")?,
        "publishedAt": text(r, "published_at")?,
        "fetchedAt": text(r, "fetched_at")?,
        "kind": kind,
    }))
}

/// `_tiles_from`: the saved Markets tile row, or `None` when it has
/// never been saved -- which is not the same as an empty row.
pub fn tiles_from(raw: &str) -> Value {
    if raw.is_empty() {
        return Value::Null;
    }
    let parsed: Value = match serde_json::from_str(raw) { Ok(v) => v, Err(_) => return Value::Null };
    let rows = match parsed.as_array() { Some(a) => a.clone(), None => vec![] };
    let mut out = Vec::new();
    for r in rows {
        if !r.is_object() {
            continue;
        }
        let sym = bagholder_model::value::field_s(&r, "symbol").trim().to_string();
        if sym.is_empty() {
            continue;
        }
        out.push(json!({
            "symbol": sym.to_uppercase(),
            "exchange": bagholder_model::value::field_s(&r, "exchange").trim().to_uppercase(),
        }));
    }
    Value::Array(out)
}

/// `_universes`.
fn universes(conn: &Connection) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare("SELECT * FROM universes ORDER BY key, value DESC, symbol")?;
    let mut rows = stmt.query([])?;
    let mut out: Map<String, Value> = Map::new();
    while let Some(r) = rows.next()? {
        let key = text(r, "key")?;
        let rec = json!({
            "symbol": text(r, "symbol")?,
            "name": text(r, "name")?,
            "value": real(r, "value")?,
            "percentChange": real(r, "percent_change")?,
            "sector": text(r, "sector")?,
            "country": text(r, "country")?,
            "fetchedAt": text(r, "fetched_at")?,
        });
        out.entry(key).or_insert_with(|| Value::Array(vec![])).as_array_mut().unwrap().push(rec);
    }
    Ok(out)
}

fn collect<F>(conn: &Connection, sql: &str, f: F) -> Result<Vec<Value>>
where
    F: Fn(&Row) -> rusqlite::Result<Value>,
{
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(f(r)?);
    }
    Ok(out)
}

/// `snapshot`.
pub fn snapshot(conn: &Connection, with_activities: bool) -> Result<Value> {
    let activities = if with_activities { all_activities(conn)? } else { vec![] };

    let accounts = collect(conn, "SELECT * FROM accounts ORDER BY id", |r| {
        Ok(json!({
            "id": text(r, "id")?,
            "nickname": text(r, "nickname")?,
            "unifiedAccountType": text(r, "unified_account_type")?,
            "currency": text(r, "currency")?,
            "status": text(r, "status")?,
            "type": text(r, "type")?,
            "netLiquidationValue": real(r, "net_liquidation_value")?,
            "marginAccountId": text(r, "margin_account_id")?,
        }))
    })?;

    // no ORDER BY: the rowid order is the order
    let balances = collect(conn, "SELECT * FROM balances", |r| {
        Ok(json!({
            "accountId": opt_text(r, "account_id")?,
            "custodianAccountId": opt_text(r, "custodian_account_id")?,
            "securityId": opt_text(r, "security_id")?,
            "quantity": real(r, "quantity")?,
        }))
    })?;

    let margin = collect(conn, "SELECT * FROM margin ORDER BY account_id", |r| {
        let currency = { let c = text(r, "currency")?; if c.is_empty() { "CAD".to_string() } else { c } };
        Ok(json!({
            "accountId": text(r, "account_id")?,
            "buyingPower": real(r, "buying_power")?,
            "currency": currency,
            "unavailable": text(r, "unavailable")?,
            "fetchedAt": text(r, "fetched_at")?,
        }))
    })?;

    let mut nav: Vec<Value> = Vec::new();
    let mut nav_by_account: Map<String, Value> = Map::new();
    {
        let mut stmt = conn.prepare("SELECT * FROM nav_history ORDER BY account_id, date")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let currency = { let c = text(r, "currency")?; if c.is_empty() { "CAD".to_string() } else { c } };
            let mut rec = Map::new();
            rec.insert("date".into(), json!(text(r, "date")?));
            rec.insert("equity".into(), real(r, "equity")?);
            rec.insert("currency".into(), json!(currency));
            if let Some(d) = r.get::<_, Option<f64>>("net_deposits")? {
                rec.insert("netDeposits".into(), json!(d));
            }
            let aid = text(r, "account_id")?;
            if aid.is_empty() {
                nav.push(Value::Object(rec));
            } else {
                nav_by_account
                    .entry(aid)
                    .or_insert_with(|| Value::Array(vec![]))
                    .as_array_mut()
                    .unwrap()
                    .push(Value::Object(rec));
            }
        }
    }

    let synced = get_meta(conn, "synced_at", "")?;
    let groups_raw = get_meta(conn, "trade_groups", "")?;
    let groups = if groups_raw.is_empty() {
        vec![]
    } else {
        match serde_json::from_str::<Value>(&groups_raw) {
            Ok(v) => clean_trade_groups(Some(&v)),
            Err(_) => vec![],
        }
    };
    let notes_raw = get_meta(conn, "trade_notes", "")?;
    let notes = if notes_raw.is_empty() {
        Map::new()
    } else {
        match serde_json::from_str::<Value>(&notes_raw) {
            Ok(v) => clean_trade_notes(Some(&v)),
            Err(_) => Map::new(),
        }
    };

    let securities = collect(conn, "SELECT * FROM securities ORDER BY id", security_from_row)?;

    let mut exposures: Map<String, Value> = Map::new();
    {
        let mut stmt = conn.prepare("SELECT * FROM exposures")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            exposures.insert(text(r, "key")?, exposure_from_row(r)?);
        }
    }

    Ok(json!({
        "activities": activities,
        "accounts": accounts,
        "balances": balances,
        "margin": margin,
        "exposures": exposures,
        "watchlist": collect(conn, "SELECT * FROM watchlist ORDER BY added_at, symbol", watch_from_row)?,
        "news": collect(conn, "SELECT * FROM news ORDER BY published_at DESC, id", news_from_row)?,
        "universes": universes(conn)?,
        "navHistory": nav,
        "navByAccount": nav_by_account,
        "syncedAt": synced,
        "tradeGroups": groups,
        "notes": notes,
        "tiles": tiles_from(&get_meta(conn, TILES_META, "")?),
        "securities": securities,
    }))
}

/// `journal`: the per-trade entries the page writes.
pub fn journal(conn: &Connection) -> Result<Map<String, Value>> {
    let raw = get_meta(conn, crate::tables::JOURNAL_META, "")?;
    if raw.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&raw) {
        Ok(v) => Ok(clean_journal(Some(&v))),
        Err(_) => Ok(Map::new()),
    }
}

/// `_clean_journal_entry` and `_clean_journal`.
pub fn clean_journal(raw: Option<&Value>) -> Map<String, Value> {
    let m = match raw { Some(Value::Object(m)) => m, _ => return Map::new() };
    let mut out = Map::new();
    for (key, val) in m {
        let kid = key.trim().to_string();
        if kid.is_empty() {
            continue;
        }
        if let Some(entry) = clean_journal_entry(val) {
            out.insert(kid, entry);
        }
    }
    out
}

/// `_clean_journal_entry`. Tags arrive as a list, or as the comma-
/// separated string an older page wrote.
fn clean_journal_entry(val: &Value) -> Option<Value> {
    let o = val.as_object()?;
    let thesis = bagholder_model::value::s(o.get("thesis"));
    let mut grade = bagholder_model::value::s(o.get("grade")).trim().to_uppercase();
    if !matches!(grade.as_str(), "A" | "B" | "C" | "F") {
        grade = String::new();
    }
    let raw: Vec<Value> = match o.get("tags") {
        Some(Value::String(s)) => s.split(',').map(|t| json!(t)).collect(),
        Some(Value::Array(a)) => a.clone(),
        _ => vec![],
    };
    let mut tags: Vec<String> = Vec::new();
    for t in raw {
        let s = bagholder_model::value::s(Some(&t)).trim().to_string();
        if !s.is_empty() && !tags.contains(&s) {
            tags.push(s);
        }
    }
    if thesis.is_empty() && grade.is_empty() && tags.is_empty() {
        return None;
    }
    Some(json!({"thesis": thesis, "tags": tags, "grade": grade}))
}
