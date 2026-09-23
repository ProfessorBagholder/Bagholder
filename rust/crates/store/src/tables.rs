//! The tables the model reads beside the activities: accounts, balances,
//! margin, the NAV history, the FX and benchmark series, and the two maps the
//! journal is kept in.

use rusqlite::{Connection, Result};
use serde_json::{json, Map, Value};

use bagholder_model::value::{field_s, get, num, s as vs};

pub const FX_PAIR: &str = "USDCAD";
pub const BENCHMARK_SYMBOL: &str = "SP500";
pub const JOURNAL_META: &str = "journal_v2";

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

fn either_val<'a>(row: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    match get(row, camel) {
        Some(v) => Some(v),
        None => get(row, snake),
    }
}

fn arr(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        _ => vec![],
    }
}

// --------------------------------------------------------------------------
// meta
// --------------------------------------------------------------------------

/// `get_meta`.
pub fn get_meta(conn: &Connection, key: &str, default: &str) -> Result<String> {
    let v: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?", [key], |r| r.get(0))
        .ok()
        .flatten();
    Ok(v.unwrap_or_else(|| default.to_string()))
}

/// `set_meta`.
pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

// --------------------------------------------------------------------------
// accounts, balances, margin
// --------------------------------------------------------------------------

/// `replace_accounts`: the account list, replaced whole. A row with no
/// id is not an account.
pub fn replace_accounts(conn: &Connection, accounts: &[crate::broker::Account]) -> Result<()> {
    crate::atomically(conn, || {
        crate::gens::replace_if_changed(conn, "SELECT id, nickname, unified_account_type, currency, status, type, net_liquidation_value, margin_account_id FROM accounts", &[], || {
        conn.execute("DELETE FROM accounts", [])?;
        for acc in accounts {
            if acc.id.is_empty() {
                continue;
            }
            conn.execute(
                "INSERT INTO accounts (id, nickname, unified_account_type, currency, status, type, net_liquidation_value, margin_account_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    acc.id,
                    acc.nickname,
                    acc.unified_account_type,
                    acc.currency,
                    acc.status,
                    acc.kind,
                    acc.net_liquidation_value,
                    acc.margin_account_id,
                ],
            )?;
        }
        Ok(())
        })?;
        Ok(())
    })
}

/// `replace_balances`.
pub fn replace_balances(conn: &Connection, balances: &[crate::broker::Balance]) -> Result<()> {
    crate::atomically(conn, || {
        crate::gens::replace_if_changed(conn, "SELECT account_id, custodian_account_id, security_id, quantity FROM balances", &[], || {
        conn.execute("DELETE FROM balances", [])?;
        for b in balances {
            conn.execute(
                "INSERT INTO balances (account_id, custodian_account_id, security_id, quantity) VALUES (?, ?, ?, ?)",
                rusqlite::params![b.account_id, b.custodian_account_id, b.security_id, b.quantity],
            )?;
        }
        Ok(())
        })?;
        Ok(())
    })
}

/// `replace_margin`: Wealthsimple's margin figures per account, replaced
/// whole on every read -- buying power with its currency, or the reason it was
/// unavailable. An account that answers nothing is not a row.
pub fn replace_margin(conn: &Connection, rows: &[crate::broker::Margin], now: &str) -> Result<()> {
    crate::atomically(conn, || {
        let changed = crate::gens::replace_if_changed(conn, "SELECT account_id, buying_power, currency, unavailable FROM margin", &[], || {
        conn.execute("DELETE FROM margin", [])?;
        for m in rows {
            if m.account_id.is_empty() {
                continue;
            }
            let currency = if m.currency.is_empty() { "CAD".to_string() } else { m.currency.clone() };
            let fetched = if m.fetched_at.is_empty() { now.to_string() } else { m.fetched_at.clone() };
            conn.execute(
                "INSERT INTO margin (account_id, buying_power, currency, unavailable, fetched_at) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![m.account_id, m.buying_power, currency, m.unavailable, fetched],
            )?;
        }
        Ok(())
        })?;
        if !changed {
            // the same figures, read again: only when they were read moves
            conn.execute("UPDATE margin SET fetched_at = ?", [now])?;
        }
        Ok(())
    })
}

// --------------------------------------------------------------------------
// NAV
// --------------------------------------------------------------------------

/// `_write_nav_points`: a point with no date or no equity is not a
/// point; an existing day is updated rather than duplicated.
fn write_nav_points(conn: &Connection, points: &[crate::broker::NavPoint]) -> Result<()> {
    crate::atomically(conn, || {
        for rec in points {
            let day: String = rec.date.chars().take(10).collect();
            if day.is_empty() {
                continue;
            }
            let equity = match rec.equity { Some(e) => e, None => continue };
            let currency = if rec.currency.is_empty() { "CAD".to_string() } else { rec.currency.clone() };
            conn.execute(
                "INSERT INTO nav_history (account_id, date, equity, currency, net_deposits) VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT(account_id, date) DO UPDATE SET equity = excluded.equity, currency = excluded.currency, net_deposits = excluded.net_deposits",
                rusqlite::params![rec.account_id, day, equity, currency, rec.net_deposits],
            )?;
        }
        Ok(())
    })
}

/// `upsert_nav`: insert or update daily values, deleting no day.
pub fn upsert_nav(conn: &Connection, points: &[crate::broker::NavPoint]) -> Result<()> {
    write_nav_points(conn, points)
}

/// `replace_nav`.
pub fn replace_nav(conn: &Connection, points: &[crate::broker::NavPoint]) -> Result<()> {
    conn.execute("DELETE FROM nav_history", [])?;
    write_nav_points(conn, points)
}

/// `nav_last_dates`: the newest stored day per account. The empty string
/// is the identity-wide series.
pub fn nav_last_dates(conn: &Connection) -> Result<std::collections::BTreeMap<String, String>> {
    let mut stmt = conn.prepare("SELECT account_id, MAX(date) AS last FROM nav_history GROUP BY account_id")?;
    let mut rows = stmt.query([])?;
    let mut out = std::collections::BTreeMap::new();
    while let Some(r) = rows.next()? {
        let last: Option<String> = r.get(1)?;
        let last = match last { Some(l) if !l.is_empty() => l, _ => continue };
        let aid: Option<String> = r.get(0)?;
        out.insert(aid.unwrap_or_default(), last);
    }
    Ok(out)
}

/// `_nav_point_from_row`: `netDeposits` is present only when the row has
/// one, because a missing figure is not zero.
pub fn nav_history(conn: &Connection, account_id: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT date, equity, currency, net_deposits FROM nav_history WHERE account_id = ? ORDER BY date",
    )?;
    let mut rows = stmt.query([account_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let currency: Option<String> = r.get(2)?;
        let currency = currency.filter(|c| !c.is_empty()).unwrap_or_else(|| "CAD".into());
        let mut rec = serde_json::Map::new();
        rec.insert("date".into(), json!(r.get::<_, Option<String>>(0)?));
        rec.insert("equity".into(), match r.get::<_, Option<f64>>(1)? {
            Some(v) => json!(v),
            None => Value::Null,
        });
        rec.insert("currency".into(), json!(currency));
        if let Some(d) = r.get::<_, Option<f64>>(3)? {
            rec.insert("netDeposits".into(), json!(d));
        }
        out.push(Value::Object(rec));
    }
    Ok(out)
}

// --------------------------------------------------------------------------
// FX and benchmark series
// --------------------------------------------------------------------------

/// `_clean_date_map`: an ISO day mapped to a positive number, and
/// nothing else.
pub fn clean_date_map(raw: Option<&Value>) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::new();
    let m = match raw { Some(Value::Object(m)) => m, _ => return out };
    for (key, val) in m {
        let d: String = key.trim().chars().take(10).collect();
        let b = d.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            continue;
        }
        match opt_num(Some(val)) {
            Some(v) if v > 0.0 => out.push((d, v)),
            _ => continue,
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn date_series(conn: &Connection, sql: &str, key: &str) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([key])?;
    let mut out = Map::new();
    while let Some(r) = rows.next()? {
        out.insert(r.get::<_, String>(0)?, json!(r.get::<_, f64>(1)?));
    }
    Ok(out)
}

pub fn fx_rates(conn: &Connection, pair: &str) -> Result<Map<String, Value>> {
    date_series(conn, "SELECT date, rate FROM fx_rates WHERE pair = ? ORDER BY date", pair)
}

pub fn fx_last_date(conn: &Connection, pair: &str) -> Result<String> {
    let d: Option<String> = conn.query_row("SELECT MAX(date) FROM fx_rates WHERE pair = ?", [pair], |r| r.get(0))?;
    Ok(d.unwrap_or_default())
}

/// `upsert_fx_rates`: `INSERT OR IGNORE`, so a rate already stored for a
/// day is never rewritten.
pub fn upsert_fx_rates(conn: &Connection, mapping: Option<&Value>, pair: &str) -> Result<usize> {
    crate::atomically(conn, || {
        let clean = clean_date_map(mapping);
        if clean.is_empty() {
            return Ok(0);
        }
        for (d, v) in &clean {
            conn.execute(
                "INSERT OR IGNORE INTO fx_rates(pair, date, rate) VALUES (?, ?, ?)",
                rusqlite::params![pair, d, v],
            )?;
        }
        Ok(clean.len())
    })
}

pub fn benchmark_prices(conn: &Connection, symbol: &str) -> Result<Map<String, Value>> {
    date_series(conn, "SELECT date, close FROM benchmark_prices WHERE symbol = ? ORDER BY date", symbol)
}

/// `benchmark_days`: how many days that index actually traded between
/// two dates, so a statutory holiday is not counted as a day of trading.
pub fn benchmark_days(conn: &Connection, symbol: &str, start: &str, end: &str) -> Result<i64> {
    let s: String = start.chars().take(10).collect();
    let e: String = end.chars().take(10).collect();
    conn.query_row(
        "SELECT COUNT(*) FROM benchmark_prices WHERE symbol = ? AND date >= ? AND date <= ?",
        rusqlite::params![symbol, s, e],
        |r| r.get(0),
    )
}

pub fn benchmark_last_date(conn: &Connection, symbol: &str) -> Result<String> {
    let d: Option<String> =
        conn.query_row("SELECT MAX(date) FROM benchmark_prices WHERE symbol = ?", [symbol], |r| r.get(0))?;
    Ok(d.unwrap_or_default())
}

pub fn upsert_benchmark_prices(conn: &Connection, mapping: Option<&Value>, symbol: &str) -> Result<usize> {
    crate::atomically(conn, || {
        let clean = clean_date_map(mapping);
        if clean.is_empty() {
            return Ok(0);
        }
        for (d, v) in &clean {
            conn.execute(
                "INSERT OR IGNORE INTO benchmark_prices(symbol, date, close) VALUES (?, ?, ?)",
                rusqlite::params![symbol, d, v],
            )?;
        }
        Ok(clean.len())
    })
}

// --------------------------------------------------------------------------
// the journal: saved trade groups and the notes on them
// --------------------------------------------------------------------------

/// `_clean_trade_groups`: a group needs an id and at least one member,
/// members are deduplicated, and the first group to claim an id keeps it.
pub fn clean_trade_groups(raw: Option<&Value>) -> Vec<Value> {
    let items = match raw { Some(Value::Array(a)) => a.clone(), _ => return vec![] };
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for item in items {
        if !item.is_object() {
            continue;
        }
        let gid = field_s(&item, "id").trim().to_string();
        let members = match item.get("members") { Some(Value::Array(m)) => m.clone(), _ => continue };
        if gid.is_empty() || seen.contains(&gid) {
            continue;
        }
        let mut keys: Vec<String> = Vec::new();
        for m in members {
            let k = vs(Some(&m)).trim().to_string();
            if k.is_empty() || keys.contains(&k) {
                continue;
            }
            keys.push(k);
        }
        if keys.is_empty() {
            continue;
        }
        seen.push(gid.clone());
        let locked = item.get("locked").map(truthy).unwrap_or(false);
        out.push(json!({"id": gid, "locked": locked, "members": keys}));
    }
    out
}

/// Truthiness: null, false, zero, and an empty string, array or object are false.
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

pub fn trade_groups(conn: &Connection) -> Result<Vec<Value>> {
    let raw = get_meta(conn, "trade_groups", "")?;
    if raw.is_empty() {
        return Ok(vec![]);
    }
    match serde_json::from_str::<Value>(&raw) {
        Ok(v) => Ok(clean_trade_groups(Some(&v))),
        Err(_) => Ok(vec![]),
    }
}

pub fn save_trade_groups(conn: &Connection, groups: Option<&Value>) -> Result<Vec<Value>> {
    let clean = clean_trade_groups(groups);
    set_meta(conn, "trade_groups", &json_text(&Value::Array(clean.clone())))?;
    Ok(clean)
}

/// `_clean_trade_notes`: a note with nothing in it is not a note, and a
/// grade outside A/B/C/F is no grade.
pub fn clean_trade_notes(raw: Option<&Value>) -> Map<String, Value> {
    let mut out = Map::new();
    let m = match raw { Some(Value::Object(m)) => m, _ => return out };
    for (key, val) in m {
        let kid = key.trim().to_string();
        if kid.is_empty() || !val.is_object() {
            continue;
        }
        let thesis = field_s(val, "thesis");
        let tag = field_s(val, "tag");
        let mut grade = field_s(val, "grade");
        if !matches!(grade.as_str(), "A" | "B" | "C" | "F") {
            grade = String::new();
        }
        if thesis.is_empty() && tag.is_empty() && grade.is_empty() {
            continue;
        }
        out.insert(kid.clone(), json!({"thesis": thesis, "tag": tag, "grade": grade, "tradeId": kid}));
    }
    out
}

pub fn trade_notes(conn: &Connection) -> Result<Map<String, Value>> {
    let raw = get_meta(conn, "trade_notes", "")?;
    if raw.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&raw) {
        Ok(v) => Ok(clean_trade_notes(Some(&v))),
        Err(_) => Ok(Map::new()),
    }
}

pub fn save_trade_notes(conn: &Connection, notes: Option<&Value>) -> Result<Map<String, Value>> {
    let clean = clean_trade_notes(notes.filter(|v| v.is_object()));
    set_meta(conn, "trade_notes", &json_text(&Value::Object(clean.clone())))?;
    Ok(clean)
}

/// The accounts and balances as the model's snapshot wants them.
pub fn accounts(conn: &Connection) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, nickname, unified_account_type, currency, status, type, net_liquidation_value, margin_account_id FROM accounts ORDER BY id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(json!({
            "id": r.get::<_, Option<String>>(0)?.unwrap_or_default(),
            "nickname": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            "unifiedAccountType": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            "currency": r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            "status": r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            "type": r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            "netLiquidationValue": match r.get::<_, Option<f64>>(6)? { Some(v) => json!(v), None => Value::Null },
            "marginAccountId": r.get::<_, Option<String>>(7)?.unwrap_or_default(),
        }));
    }
    Ok(out)
}

pub fn balances(conn: &Connection) -> Result<Vec<Value>> {
    let mut stmt =
        conn.prepare("SELECT account_id, custodian_account_id, security_id, quantity FROM balances ORDER BY id")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(json!({
            "accountId": r.get::<_, Option<String>>(0)?.unwrap_or_default(),
            "custodianAccountId": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            "securityId": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            "quantity": match r.get::<_, Option<f64>>(3)? { Some(v) => json!(v), None => Value::Null },
        }));
    }
    Ok(out)
}

pub fn margin(conn: &Connection) -> Result<Vec<Value>> {
    let mut stmt = conn
        .prepare("SELECT account_id, buying_power, currency, unavailable, fetched_at FROM margin ORDER BY account_id")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(json!({
            "accountId": r.get::<_, Option<String>>(0)?.unwrap_or_default(),
            "buyingPower": match r.get::<_, Option<f64>>(1)? { Some(v) => json!(v), None => Value::Null },
            "currency": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            "unavailable": r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            "fetchedAt": r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        }));
    }
    Ok(out)
}

/// Kept for callers holding a raw array of rows.
pub fn as_rows(v: Option<&Value>) -> Vec<Value> {
    arr(v)
}

/// Kept so callers can read either spelling without importing the helper.
pub fn field_either(row: &Value, camel: &str, snake: &str) -> Option<Value> {
    either_val(row, camel, snake).cloned()
}

// --------------------------------------------------------------------------
// JSON as the database stores it
// --------------------------------------------------------------------------

/// Compact JSON with `", "` and `": "` separators, and every
/// non-ASCII character escaped.
///
/// These values are stored in `meta` and read back by whichever implementation
/// is running, so writing them the other way would rewrite the user's database
/// on the first save for no reason and make the two byte-different for the
/// same content.
pub fn json_text(v: &Value) -> String {
    let mut out = String::new();
    write_py_json(v, &mut out);
    out
}

fn write_py_json(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_py_str(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_py_json(x, out);
            }
            out.push(']');
        }
        Value::Object(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_py_str(k, out);
                out.push_str(": ");
                write_py_json(x, out);
            }
            out.push('}');
        }
    }
}

fn write_py_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) < 0x7f => out.push(c),
            // ensure_ascii: anything above ASCII goes out as an escape, and
            // anything outside the basic plane as a surrogate pair.
            c => {
                let cp = c as u32;
                if cp > 0xffff {
                    let v = cp - 0x10000;
                    out.push_str(&format!("\\u{:04x}\\u{:04x}", 0xd800 + (v >> 10), 0xdc00 + (v & 0x3ff)));
                } else {
                    out.push_str(&format!("\\u{:04x}", cp));
                }
            }
        }
    }
    out.push('"');
}

/// `json_text` with sorted keys: the same formatting, with every object's
/// keys in order. A ticket's stored request uses it so the same order is the
/// same text whichever way it was built.
pub fn json_text_sorted(v: &Value) -> String {
    json_text(&sorted(v))
}

fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                out.insert(k.clone(), sorted(&m[k]));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
        other => other.clone(),
    }
}
