//! The rest of the store: the securities table, the journal writers, the
//! saved tile row, the pull schedule, and the wipe the Data & storage dialog
//! performs.

use rusqlite::{Connection, Result};
use serde_json::Value;
use std::collections::BTreeMap;

use bagholder_model::input::{Journal, JournalEntry, TileRef};
use bagholder_model::securities::Security;

/// `SYNC_META_KEYS`: the bookmarks a wipe clears so the next sync starts
/// from zero.
pub const SYNC_META_KEYS: [&str; 3] = ["synced_at", "last_activity_pull", "security_id_backfill_done"];

/// `ACTIVITY_PULL_WEEKDAYS` / `_HOUR` / `_MINUTE`: one pull per weekday,
/// after the market has closed.
pub const ACTIVITY_PULL_HOUR: u32 = 14;
pub const ACTIVITY_PULL_MINUTE: u32 = 0;

/// `upsert_securities`. Nothing passes a per-row `fetchedAt` today (the
/// caller stamps them all at once), but a row that carries one keeps it.
pub fn upsert_securities(conn: &Connection, rows: &[bagholder_model::securities::Security], now: &str) -> Result<()> {
    crate::atomically(conn, || {
        for sec in rows {
            let sid = sec.id.trim().to_string();
            if sid.is_empty() {
                continue;
            }
            let under = sec.underlying_id.trim().to_string();
            conn.execute(
                "INSERT INTO securities (id, symbol, name, primary_exchange, primary_mic, currency, underlying_id, fetched_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(id) DO UPDATE SET symbol = excluded.symbol, name = excluded.name, \
                 primary_exchange = excluded.primary_exchange, primary_mic = excluded.primary_mic, \
                 currency = excluded.currency, underlying_id = excluded.underlying_id, fetched_at = excluded.fetched_at",
                rusqlite::params![
                    sid,
                    sec.symbol,
                    sec.name,
                    sec.primary_exchange,
                    sec.primary_mic,
                    sec.currency,
                    if under.is_empty() { None } else { Some(under) },
                    now,
                ],
            )?;
        }
        Ok(())
    })
}

pub fn list_securities(conn: &Connection) -> Result<Vec<Security>> {
    crate::rows::securities(conn)
}

/// `missing_security_ids`: the ids the table does not hold, in the order
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

/// `needs_security_id_backfill`: whether any broker row with a symbol
/// still has no security id.
pub fn needs_security_id_backfill(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM activities WHERE source = 'wealthsimple' AND IFNULL(symbol, '') != '' AND (security_id IS NULL OR security_id = '') LIMIT 1)",
        [],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// A raw entry validated: a grade outside A/B/C/F is no grade, and an entry
/// left with no thesis, grade or tags is no entry at all.
fn clean_journal_entry(e: &JournalEntry) -> Option<JournalEntry> {
    let mut grade = e.grade.trim().to_uppercase();
    if !matches!(grade.as_str(), "A" | "B" | "C" | "F") {
        grade = String::new();
    }
    let mut tags: Vec<String> = Vec::new();
    for t in &e.tags {
        let s = t.trim().to_string();
        if !s.is_empty() && !tags.contains(&s) {
            tags.push(s);
        }
    }
    if e.thesis.is_empty() && grade.is_empty() && tags.is_empty() {
        return None;
    }
    Some(JournalEntry { thesis: e.thesis.clone(), tags, grade })
}

/// A raw journal, validated: a blank-trimmed key holds nothing, as does an
/// invalid entry.
fn clean_journal(raw: &BTreeMap<String, JournalEntry>) -> Journal {
    let mut out = Journal::new();
    for (key, val) in raw {
        let kid = key.trim().to_string();
        if kid.is_empty() {
            continue;
        }
        if let Some(clean) = clean_journal_entry(val) {
            out.insert(kid, clean);
        }
    }
    out
}

/// The journal as the stored text holds it, read leniently and validated.
pub fn journal(conn: &Connection) -> Result<Journal> {
    let raw = crate::tables::get_meta(conn, crate::tables::JOURNAL_META, "")?;
    if raw.is_empty() {
        return Ok(Journal::new());
    }
    let parsed: BTreeMap<String, JournalEntry> = match serde_json::from_str::<Value>(&raw) {
        Ok(v) => bagholder_model::lenient::objmap(&v),
        Err(_) => BTreeMap::new(),
    };
    Ok(clean_journal(&parsed))
}

/// Sorted, so the stored text is the same byte for byte whichever order the
/// entries arrived in.
fn write_journal(conn: &Connection, j: &Journal) -> Result<()> {
    let sorted: BTreeMap<&String, &JournalEntry> = j.iter().collect();
    let text = crate::tables::json_text(&serde_json::to_value(&sorted).unwrap_or(Value::Null));
    crate::tables::set_meta(conn, crate::tables::JOURNAL_META, &text)
}

/// `save_journal`.
pub fn save_journal(conn: &Connection, entries: &Journal) -> Result<Journal> {
    let mut clean = Journal::new();
    for (key, val) in entries {
        let kid = key.trim().to_string();
        if kid.is_empty() {
            continue;
        }
        if let Some(c) = clean_journal_entry(val) {
            clean.insert(kid, c);
        }
    }
    write_journal(conn, &clean)?;
    Ok(clean)
}

/// `save_journal_entry`: merge one entry. An entry with no thesis, grade
/// or tags deletes the key.
pub fn save_journal_entry(conn: &Connection, key: &str, entry: Option<&JournalEntry>) -> Result<Journal> {
    let kid = key.trim().to_string();
    let mut current = journal(conn)?;
    if kid.is_empty() {
        return Ok(current);
    }
    match entry.and_then(clean_journal_entry) {
        Some(clean) => { current.insert(kid, clean); }
        None => { current.remove(&kid); }
    }
    write_journal(conn, &current)?;
    Ok(current)
}

/// `save_tiles`: the Markets tile row, in order. Saving it is what the
/// tab's plus, cross and drag do.
pub fn save_tiles(conn: &Connection, rows: &[TileRef]) -> Result<Vec<TileRef>> {
    let mut clean: Vec<TileRef> = Vec::new();
    for r in rows {
        let sym = r.symbol.trim().to_uppercase();
        if sym.is_empty() {
            continue;
        }
        clean.push(TileRef { symbol: sym, exchange: r.exchange.trim().to_uppercase() });
    }
    let text = crate::tables::json_text(&serde_json::to_value(&clean).unwrap_or(Value::Array(vec![])));
    crate::tables::set_meta(conn, crate::rows::TILES_META, &text)?;
    Ok(clean)
}

/// Seconds from `now_unix` until the next pull window opens (the next weekday's
/// pull time, local). What the sync loop sleeps until, instead of asking every half
/// minute whether it is time yet. Zero when the zone is unknown.
pub fn seconds_until_pull_window(now_unix: i64) -> i64 {
    let (y, m, d, hh, mm) = match local_parts(now_unix) { Some(p) => p, None => return 0 };
    let minute_now = (hh * 60 + mm) as i64;
    let window = (ACTIVITY_PULL_HOUR * 60 + ACTIVITY_PULL_MINUTE) as i64;
    let today = weekday_of(y, m, d) as i64; // 0 = Monday
    for ahead in 0..8 {
        let weekday = (today + ahead) % 7;
        let minutes = ahead * 1440 + window - minute_now;
        if weekday <= 4 && minutes > 0 {
            return minutes * 60 - now_unix.rem_euclid(60);
        }
    }
    0
}

/// `activity_pull_due`: due at 2:00 PM Mountain, Monday to Friday, after
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

/// Monday is 0.
fn weekday_of(y: i64, m: u32, d: u32) -> u32 {
    let days = bagholder_model::dates::to_days(y, m, d);
    // 1970-01-01 was a Thursday, which is 3
    (((days + 3) % 7 + 7) % 7) as u32
}

/// `clear_synced_data`: wipe everything the sync wrote so the next one
/// starts from zero.
///
/// The journal and the downloaded market data are kept unless told otherwise.
/// The Wealthsimple login is not this function's business.
pub fn clear_synced_data(conn: &Connection, keep_journal: bool, keep_market: bool) -> Result<()> {
    crate::atomically(conn, || {
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
    })
}
