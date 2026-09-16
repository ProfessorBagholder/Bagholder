//! Taking rows in: the Wealthsimple sync, and the CSV or typed-in merge.
//!
//! A Wealthsimple row is known by its canonical id, and a known row is
//! replaced only when Wealthsimple itself has revised it. Bagholder never
//! edits a row of the broker's on its own. An imported row that matches one
//! stored fill exactly is linked to it rather than duplicated; two matches are
//! not guessed between.

use rusqlite::{Connection, Result};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

use bagholder_model::value::{field_s, get, num};

use crate::activities::{
    activity_by_id, all_activities, canonical_ids, field_match_key, insert_activity, insert_local,
    is_real_account, link_match_key, looks_like_homemade_id,
};

/// `store._REVISABLE_COLUMNS`: what Wealthsimple revises on a row of its own.
///
/// A dividend announced as a placeholder on the record date -- no cash, dated
/// that day -- becomes the paid dividend on pay day under the same canonical
/// id. The identity, the account and the stored id stay.
pub const REVISABLE_COLUMNS: [&str; 18] = [
    "occurred_at", "transaction_date", "settlement_date", "activity_type", "activity_sub_type",
    "description", "direction", "symbol", "name", "currency", "quantity", "unit_price", "commission",
    "net_cash_amount", "category", "raw_type", "aft_type", "counter_symbol",
];

fn either(row: &Value, camel: &str, snake: &str) -> String {
    let v = field_s(row, camel);
    if v.is_empty() { field_s(row, snake) } else { v }
}

/// `store._differs`: numbers within a billionth are the same; everything else
/// compares as text, with absent reading as empty.
fn differs(a: &Value, b: &Value) -> bool {
    let numeric = a.is_f64() || b.is_f64();
    if numeric {
        let (x, y) = (num(Some(a), f64::NAN), num(Some(b), f64::NAN));
        if x.is_nan() || y.is_nan() {
            return true;
        }
        return (x - y).abs() > 1e-9;
    }
    let sa = if a.is_null() { String::new() } else { bagholder_model::value::s(Some(a)) };
    let sb = if b.is_null() { String::new() } else { bagholder_model::value::s(Some(b)) };
    sa != sb
}

/// `store.find_link_candidates`: unlinked local rows matching symbol, side,
/// quantity, price and date -- and the account when it is a real one.
pub fn find_link_candidates(conn: &Connection, act: &Value) -> Result<Vec<Value>> {
    let account = {
        let v = field_s(act, "accountId");
        if v.is_empty() { field_s(act, "account_id") } else { v }
    };
    let include_account = is_real_account(&account);
    let target = link_match_key(act, include_account);

    let mut stmt = conn.prepare(
        "SELECT id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, account_type, activity_type, activity_sub_type, description, direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, balance, source, raw_type, aft_type, counter_symbol, security_id FROM activities WHERE canonical_id IS NULL OR canonical_id = ''",
    )?;
    let rows = stmt.query_map([], crate::activities::row_to_activity)?;
    let mut out = Vec::new();
    for row in rows {
        let mapped = row?;
        if link_match_key(&mapped, include_account) == target {
            out.push(mapped);
        }
    }
    Ok(out)
}

/// `store.stamp_canonical_id`: mark a local row as the broker's, but only when
/// it has no broker id already.
///
/// The answer is whether this statement changed a row, which is not what
/// Python answers: it returns `conn.total_changes > 0`, and that counts every
/// change the pooled connection has made since it was opened, so it reads
/// true even for a row that does not exist.
///
/// That one is not cosmetic. `apply_wealthsimple_mapped` takes a true here as
/// "the imported row is now the broker's" and skips the insert. If another
/// caller stamps the same row between the search and this update, Python
/// counts a link that did not happen and the broker's row is never stored --
/// the fill is lost. The accurate answer is kept here deliberately.
pub fn stamp_canonical_id(conn: &Connection, activity_id: &str, canonical_id: &str) -> Result<bool> {
    let cid = canonical_id.trim();
    if cid.is_empty() || looks_like_homemade_id(cid) {
        return Ok(false);
    }
    let n = conn.execute(
        "UPDATE activities SET canonical_id = ? WHERE id = ? AND (canonical_id IS NULL OR canonical_id = '')",
        rusqlite::params![cid, activity_id],
    )?;
    Ok(n > 0)
}

/// The stored row's revisable columns, under their column names.
fn stored_columns(conn: &Connection, cid: &str) -> Result<Option<Map<String, Value>>> {
    let cols = crate::activities::COLUMNS;
    let sql = format!("SELECT {} FROM activities WHERE canonical_id = ?", cols.join(", "));
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([cid])?;
    let row = match rows.next()? { Some(r) => r, None => return Ok(None) };
    let mut out = Map::new();
    for (i, c) in cols.iter().enumerate() {
        let v = match row.get_ref(i)? {
            rusqlite::types::ValueRef::Null => Value::Null,
            rusqlite::types::ValueRef::Integer(n) => json!(n as f64),
            rusqlite::types::ValueRef::Real(f) => json!(f),
            rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t).to_string()),
            rusqlite::types::ValueRef::Blob(_) => Value::Null,
        };
        out.insert((*c).to_string(), v);
    }
    Ok(Some(out))
}

/// `store._revise_wealthsimple_row`: replace the stored copy with the broker's
/// current version when a revisable field changed.
pub fn revise_wealthsimple_row(conn: &Connection, cid: &str, row: &Value) -> Result<bool> {
    let incoming = crate::activities::insert_columns(row, "", Some(cid));
    let stored = match stored_columns(conn, cid)? { Some(s) => s, None => return Ok(false) };

    let mut changed: Vec<&str> = REVISABLE_COLUMNS
        .iter()
        .copied()
        .filter(|c| {
            differs(
                incoming.get(*c).unwrap_or(&Value::Null),
                stored.get(*c).unwrap_or(&Value::Null),
            )
        })
        .collect();
    if changed.is_empty() {
        return Ok(false);
    }
    // a security id the broker now supplies is taken, but never replaced
    let incoming_sid = incoming.get("security_id").cloned().unwrap_or(Value::Null);
    let stored_sid = stored.get("security_id").cloned().unwrap_or(Value::Null);
    let incoming_has = !matches!(&incoming_sid, Value::Null) && bagholder_model::value::s(Some(&incoming_sid)) != "";
    let stored_has = !matches!(&stored_sid, Value::Null) && bagholder_model::value::s(Some(&stored_sid)) != "";
    if incoming_has && !stored_has {
        changed.push("security_id");
    }

    let sets = changed.iter().map(|c| format!("{} = ?", c)).collect::<Vec<_>>().join(", ");
    let sql = format!("UPDATE activities SET {} WHERE canonical_id = ?", sets);
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = changed
        .iter()
        .map(|c| crate::activities::to_sql(incoming.get(*c).unwrap_or(&Value::Null)))
        .collect();
    params.push(Box::new(cid.to_string()));
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    conn.execute(&sql, refs.as_slice())?;
    Ok(true)
}

#[derive(Default, Debug, serde::Serialize)]
pub struct Applied {
    pub inserted: usize,
    pub linked: usize,
    pub skipped: usize,
    pub revised: usize,
}

/// `store.apply_wealthsimple_mapped`.
pub fn apply_wealthsimple_mapped(
    conn: &Connection,
    rows: &[Value],
    new_id: &dyn Fn() -> String,
) -> Result<Applied> {
    let mut out = Applied::default();
    let mut known: HashSet<String> = canonical_ids(conn)?.into_iter().collect();

    for raw in rows {
        if !truthy(raw) {
            continue;
        }
        let mut row: Map<String, Value> = match raw { Value::Object(m) => m.clone(), _ => continue };
        row.insert("source".into(), json!("wealthsimple"));
        let row = Value::Object(row);

        let cid = either(&row, "canonicalId", "canonical_id").trim().to_string();
        if cid.is_empty() || looks_like_homemade_id(&cid) {
            continue;
        }
        if known.contains(&cid) {
            if revise_wealthsimple_row(conn, &cid, &row)? {
                out.revised += 1;
                continue;
            }
            let sid = either(&row, "securityId", "security_id").trim().to_string();
            if !sid.is_empty() {
                conn.execute(
                    "UPDATE activities SET security_id = ? WHERE canonical_id = ? AND (security_id IS NULL OR security_id = '')",
                    rusqlite::params![sid, cid],
                )?;
            }
            out.skipped += 1;
            continue;
        }
        let matches = find_link_candidates(conn, &row)?;
        if matches.len() == 1 {
            let id = field_s(&matches[0], "id");
            if stamp_canonical_id(conn, &id, &cid)? {
                known.insert(cid.clone());
                out.linked += 1;
                continue;
            }
        }
        insert_activity(conn, &row, Some(&cid), None, new_id)?;
        known.insert(cid);
        out.inserted += 1;
    }
    Ok(out)
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

#[derive(Debug, serde::Serialize)]
pub struct Merged {
    pub ok: bool,
    pub added: usize,
    pub duplicates: usize,
    pub activities: Vec<Value>,
}

/// `store.merge_local_rows`: a CSV or typed merge, on date, account, symbol,
/// quantity, price and cash rather than on any id.
pub fn merge_local_rows(conn: &Connection, rows: &[Value], new_id: &dyn Fn() -> String) -> Result<Merged> {
    let mut stored: Vec<Value> = Vec::new();
    let mut added = 0usize;
    let mut duplicates = 0usize;

    let mut existing: HashMap<Key, usize> = HashMap::new();
    for a in all_activities(conn)? {
        *existing.entry(Key::of(&a)).or_insert(0) += 1;
    }

    let mut incoming_seen: HashMap<Key, usize> = HashMap::new();
    for raw in rows {
        if !truthy(raw) {
            continue;
        }
        let mut row: Map<String, Value> = match raw { Value::Object(m) => m.clone(), _ => continue };
        let mut source = field_s(raw, "source");
        if source.is_empty() {
            source = "csv".into();
        }
        if source == "wealthsimple" {
            let cid = crate::activities::canonical_from_row(raw, "wealthsimple");
            if let Some(_cid) = cid {
                let result = apply_wealthsimple_mapped(conn, std::slice::from_ref(raw), new_id)?;
                added += result.inserted + result.linked;
                if result.skipped > 0 {
                    duplicates += result.skipped;
                }
                continue;
            }
            source = "csv".into();
        }
        row.insert("source".into(), json!(source));
        row.remove("canonicalId");
        row.remove("canonical_id");
        let row = Value::Object(row);

        let k = Key::of(&row);
        let n = incoming_seen.entry(k.clone()).or_insert(0);
        *n += 1;
        if *n <= *existing.get(&k).unwrap_or(&0) {
            duplicates += 1;
            continue;
        }
        let saved = insert_local(conn, &row, new_id)?;
        *existing.entry(k).or_insert(0) += 1;
        stored.push(saved);
        added += 1;
    }
    Ok(Merged { ok: true, added, duplicates, activities: stored })
}

/// Re-exported so callers reading a single row back can use it.
pub fn by_id(conn: &Connection, id: &str) -> Result<Option<Value>> {
    activity_by_id(conn, id)
}

/// Kept for the tool: a row's revisable view, for reporting a diff.
pub fn revisable_view(row: &Value) -> Map<String, Value> {
    let cols = crate::activities::insert_columns(row, "", None);
    let mut out = Map::new();
    for c in REVISABLE_COLUMNS {
        out.insert(c.to_string(), cols.get(c).cloned().unwrap_or(Value::Null));
    }
    out
}

/// A field match key that can be a map key.
///
/// Python compares the tuple's floats by value, so the bits are used here with
/// a negative zero folded onto zero -- `0.0 == -0.0` there, and a row whose
/// cash is written either way is the same fill.
#[derive(PartialEq, Eq, Hash, Clone)]
pub struct Key(String, String, String, u64, u64, u64);

impl Key {
    pub fn of(row: &Value) -> Key {
        let (d, a, s, q, p, c) = field_match_key(row, true);
        Key(d, a, s, bits(q), bits(p), bits(c))
    }
}

fn bits(v: f64) -> u64 {
    (if v == 0.0 { 0.0 } else { v }).to_bits()
}

/// Exposed for the tool's reporting.
pub fn get_field<'a>(row: &'a Value, k: &str) -> Option<&'a Value> {
    get(row, k)
}
