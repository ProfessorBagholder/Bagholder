//! The rest of the store: the securities table, the journal writers, the
//! saved tile row, the pull schedule, and the wipe the Data & storage dialog
//! performs.

use rusqlite::{Connection, Result};
use serde_json::{json, Map, Value};

use bagholder_model::value::{field_s, get};

/// `store.SYNC_META_KEYS`: the bookmarks a wipe clears so the next sync starts
/// from zero.
pub const SYNC_META_KEYS: [&str; 3] = ["synced_at", "last_activity_pull", "security_id_backfill_done"];

/// `store.ACTIVITY_PULL_WEEKDAYS` / `_HOUR` / `_MINUTE`: one pull per weekday,
/// after the market has closed.
pub const ACTIVITY_PULL_HOUR: u32 = 14;
pub const ACTIVITY_PULL_MINUTE: u32 = 0;

fn either(row: &Value, camel: &str, snake: &str) -> String {
    let v = field_s(row, camel);
    if v.is_empty() { field_s(row, snake) } else { v }
}

/// `store.upsert_securities`.
pub fn upsert_securities(conn: &Connection, rows: &[Value], now: &str) -> Result<()> {
    for raw in rows {
        if !raw.is_object() {
            continue;
        }
        let sid = field_s(raw, "id").trim().to_string();
        if sid.is_empty() {
            continue;
        }
        // the camelCase key wins only when it is present at all, as Python's
        // `if ... is not None else ...` does
        let under = match get(raw, "underlyingId") {
            Some(v) => bagholder_model::value::s(Some(v)),
            None => field_s(raw, "underlying_id"),
        }
        .trim()
        .to_string();
        let fetched = { let f = either(raw, "fetchedAt", "fetched_at"); if f.is_empty() { now.to_string() } else { f } };
        conn.execute(
            "INSERT INTO securities (id, symbol, name, primary_exchange, primary_mic, currency, underlying_id, fetched_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET symbol = excluded.symbol, name = excluded.name, \
             primary_exchange = excluded.primary_exchange, primary_mic = excluded.primary_mic, \
             currency = excluded.currency, underlying_id = excluded.underlying_id, fetched_at = excluded.fetched_at",
            rusqlite::params![
                sid,
                field_s(raw, "symbol"),
                field_s(raw, "name"),
                either(raw, "primaryExchange", "primary_exchange"),
                either(raw, "primaryMic", "primary_mic"),
                field_s(raw, "currency"),
                if under.is_empty() { None } else { Some(under) },
                fetched,
            ],
        )?;
    }
    Ok(())
}

pub fn list_securities(conn: &Connection) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM securities ORDER BY id")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let uid: Option<String> = r.get("underlying_id")?;
        out.push(json!({
            "id": r.get::<_, String>("id")?,
            "symbol": r.get::<_, Option<String>>("symbol")?.unwrap_or_default(),
            "name": r.get::<_, Option<String>>("name")?.unwrap_or_default(),
            "primaryExchange": r.get::<_, Option<String>>("primary_exchange")?.unwrap_or_default(),
            "primaryMic": r.get::<_, Option<String>>("primary_mic")?.unwrap_or_default(),
            "currency": r.get::<_, Option<String>>("currency")?.unwrap_or_default(),
            "underlyingId": match uid { Some(u) if !u.is_empty() => json!(u), _ => Value::Null },
        }));
    }
    Ok(out)
}

/// `store.missing_security_ids`: the ids the table does not hold, in the order
/// they were asked for, four hundred to a query.
pub fn missing_security_ids(conn: &Connection, ids: &[String]) -> Result<Vec<String>> {
    let mut wanted: Vec<String> = Vec::new();
    for raw in ids {
        let sid = raw.trim().to_string();
        if sid.is_empty() || wanted.contains(&sid) {
            continue;
        }
        wanted.push(sid);
    }
    if wanted.is_empty() {
        return Ok(vec![]);
    }
    let mut have: Vec<String> = Vec::new();
    for chunk in wanted.chunks(400) {
        let marks = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT id FROM securities WHERE id IN ({})", marks);
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let mut rows = stmt.query(params.as_slice())?;
        while let Some(r) = rows.next()? {
            have.push(r.get::<_, String>(0)?);
        }
    }
    Ok(wanted.into_iter().filter(|s| !have.contains(s)).collect())
}

/// `store.needs_security_id_backfill`: whether any broker row with a symbol
/// still has no security id.
pub fn needs_security_id_backfill(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM activities WHERE source = 'wealthsimple' AND IFNULL(symbol, '') != '' AND (security_id IS NULL OR security_id = '') LIMIT 1)",
        [],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// `store.exposures_map`: every record keyed by what it is for -- a security
/// id, or a share or fund key.
pub fn exposures_map(conn: &Connection) -> Result<Map<String, Value>> {
    let snapshot = crate::snapshot::snapshot(conn, false)?;
    Ok(snapshot.get("exposures").and_then(|v| v.as_object()).cloned().unwrap_or_default())
}

/// `store.save_journal`.
pub fn save_journal(conn: &Connection, entries: Option<&Value>) -> Result<Map<String, Value>> {
    let clean = crate::snapshot::clean_journal(entries.filter(|v| v.is_object()));
    crate::tables::set_meta(conn, crate::tables::JOURNAL_META, &crate::tables::json_text(&Value::Object(clean.clone())))?;
    Ok(clean)
}

/// `store.save_journal_entry`: merge one entry. An entry with no thesis, grade
/// or tags deletes the key.
pub fn save_journal_entry(conn: &Connection, key: &str, entry: Option<&Value>) -> Result<Map<String, Value>> {
    let kid = key.trim().to_string();
    let mut current = crate::snapshot::journal(conn)?;
    if kid.is_empty() {
        return Ok(current);
    }
    let one = entry.map(|e| crate::snapshot::clean_journal(Some(&json!({ kid.clone(): e }))));
    match one.and_then(|m| m.get(&kid).cloned()) {
        Some(clean) => { current.insert(kid, clean); }
        None => {
            // `Map::remove` under `preserve_order` is a swap-remove: it moves
            // the last entry into the hole and the stored journal comes back
            // in a different order every time a key is deleted. The map is
            // rebuilt instead.
            let mut kept = Map::new();
            for (k, v) in current.iter() {
                if *k != kid {
                    kept.insert(k.clone(), v.clone());
                }
            }
            current = kept;
        }
    }
    crate::tables::set_meta(conn, crate::tables::JOURNAL_META, &crate::tables::json_text(&Value::Object(current.clone())))?;
    Ok(current)
}

/// `store.save_tiles`: the Markets tile row, in order. Saving it is what the
/// tab's plus, cross and drag do.
pub fn save_tiles(conn: &Connection, rows: &[Value]) -> Result<Value> {
    let raw = crate::tables::json_text(&Value::Array(rows.to_vec()));
    let clean = crate::snapshot::tiles_from(&raw);
    let clean = if clean.is_null() { Value::Array(vec![]) } else { clean };
    crate::tables::set_meta(conn, crate::snapshot::TILES_META, &crate::tables::json_text(&clean))?;
    Ok(clean)
}

/// `store.activity_pull_due`: due at 2:00 PM Mountain, Monday to Friday, after
/// the market has closed, and once per weekday.
pub fn activity_pull_due(conn: &Connection, now_unix: i64) -> Result<bool> {
    let (y, m, d, hh, mm) = match local_parts(now_unix) { Some(p) => p, None => return Ok(false) };
    let weekday = weekday_of(y, m, d);
    if weekday > 4 {
        return Ok(false);
    }
    if (hh, mm) < (ACTIVITY_PULL_HOUR, ACTIVITY_PULL_MINUTE) {
        return Ok(false);
    }
    // the close, as a local wall time on the same day
    let close_key = (y, m, d, ACTIVITY_PULL_HOUR, ACTIVITY_PULL_MINUTE);
    let last = crate::tables::get_meta(conn, "last_activity_pull", "")?;
    if last.is_empty() {
        return Ok(true);
    }
    match instant_to_local(&last) {
        None => Ok(true),
        Some(then) => Ok(then < close_key),
    }
}

pub fn mark_activity_pulled(conn: &Connection, when: &str) -> Result<()> {
    crate::tables::set_meta(conn, "last_activity_pull", when)
}

/// The app's own time zone, which is where the pull schedule is read.
fn local_parts(unix: i64) -> Option<(i64, u32, u32, u32, u32)> {
    let (date, time) = bagholder_model::clock::when_parts(&stamp(unix));
    let (y, m, d) = bagholder_model::dates::parse_iso(&date)?;
    let (hh, mm) = time.split_once(':')?;
    Some((y, m, d, hh.parse().ok()?, mm.parse().ok()?))
}

fn instant_to_local(s: &str) -> Option<(i64, u32, u32, u32, u32)> {
    let (date, time) = bagholder_model::clock::when_parts(s);
    let (y, m, d) = bagholder_model::dates::parse_iso(&date)?;
    if time.is_empty() {
        return Some((y, m, d, 0, 0));
    }
    let (hh, mm) = time.split_once(':')?;
    Some((y, m, d, hh.parse().ok()?, mm.parse().ok()?))
}

fn stamp(unix: i64) -> String {
    let days = unix.div_euclid(86400);
    let rem = unix.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// Monday is 0, as Python's `weekday()` is.
fn weekday_of(y: i64, m: u32, d: u32) -> u32 {
    let days = bagholder_model::dates::to_days(y, m, d);
    // 1970-01-01 was a Thursday, which is 3
    (((days + 3) % 7 + 7) % 7) as u32
}

/// `store.clear_synced_data`: wipe everything the sync wrote so the next one
/// starts from zero.
///
/// The journal and the downloaded market data are kept unless told otherwise.
/// The Wealthsimple login is not this function's business.
pub fn clear_synced_data(conn: &Connection, keep_journal: bool, keep_market: bool) -> Result<()> {
    for table in ["activities", "accounts", "balances", "margin", "nav_history", "securities", "grouped_trades"] {
        conn.execute(&format!("DELETE FROM {}", table), [])?;
    }
    let mut keys: Vec<String> = SYNC_META_KEYS.iter().map(|k| k.to_string()).collect();
    keys.push("trade_groups".into());
    keys.push("trade_notes".into());
    if !keep_journal {
        keys.push(crate::tables::JOURNAL_META.into());
    }
    for k in keys {
        conn.execute("DELETE FROM meta WHERE key = ?", [k])?;
    }
    if !keep_market {
        for table in [
            "fx_rates", "benchmark_prices", "distributions", "distribution_fetches", "quotes",
            "price_history", "history_fetches", "price_bars", "bar_fetches",
        ] {
            conn.execute(&format!("DELETE FROM {}", table), [])?;
        }
        conn.execute("DELETE FROM meta WHERE key IN ('spy_by_date', 'market_attempt_at')", [])?;
        for prefix in ["bars_miss:", "bars_source:", "coinbase_product:", "coingecko_id:", "tmx_form:", "yahoo_miss:"] {
            conn.execute("DELETE FROM meta WHERE key LIKE ?", [format!("{}%", prefix)])?;
        }
    }
    Ok(())
}
