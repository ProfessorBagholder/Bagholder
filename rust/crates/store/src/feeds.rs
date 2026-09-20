//! What the app reads about an instrument beside its price: exposure records,
//! the watchlist, news, filings, short selling, the published gauges, and the
//! notifications the app raises.
//!
//! Each of these is replaced per source rather than wholesale, so one feed
//! that is down does not empty what another put there. What has been read out
//! of a document survives a refresh of the list it came from.

use rusqlite::{Connection, Result, Row};
use std::collections::HashSet;
use serde_json::{json, Map, Value};

use bagholder_model::value::{field_s, get, num};

/// `NOTIFICATIONS_KEPT`.
pub const NOTIFICATIONS_KEPT: i64 = 200;

fn text(r: &Row, name: &str) -> Result<String> {
    Ok(r.get::<_, Option<String>>(name)?.unwrap_or_default())
}

fn real(r: &Row, name: &str) -> Result<Value> {
    Ok(match r.get::<_, Option<f64>>(name)? { Some(v) => json!(v), None => Value::Null })
}

fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(x) => { let n = num(Some(x), f64::NAN); if n.is_nan() { None } else { Some(n) } }
    }
}

fn up(s: &str) -> String { s.trim().to_uppercase() }

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

// --------------------------------------------------------------------------
// exposure
// --------------------------------------------------------------------------

/// `replace_exposure`: one record -- the sectors and countries as
/// `{name: fraction}`, the share of the holding they cover, and where it came
/// from.
pub fn replace_exposure(conn: &Connection, key: &str, rec: &Value, now: &str) -> Result<()> {
    let sectors = rec.get("sectors").filter(|v| v.is_object()).cloned().unwrap_or_else(|| json!({}));
    let countries = rec.get("countries").filter(|v| v.is_object()).cloned().unwrap_or_else(|| json!({}));
    conn.execute(
        "INSERT INTO exposures (key, sectors, countries, coverage, source, as_of, industry, error, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(key) DO UPDATE SET sectors = excluded.sectors, countries = excluded.countries, coverage = excluded.coverage, source = excluded.source, \
         as_of = excluded.as_of, industry = excluded.industry, error = excluded.error, fetched_at = excluded.fetched_at",
        rusqlite::params![
            key,
            crate::tables::json_text(&sectors),
            crate::tables::json_text(&countries),
            num(get(rec, "coverage"), 0.0),
            field_s(rec, "source"),
            field_s(rec, "asOf"),
            field_s(rec, "industry"),
            field_s(rec, "error"),
            now,
        ],
    )?;
    Ok(())
}

// --------------------------------------------------------------------------
// watchlist
// --------------------------------------------------------------------------

fn watch_from_row(r: &Row) -> Result<Value> {
    Ok(json!({
        "symbol": text(r, "symbol")?,
        "exchange": text(r, "exchange")?,
        "name": text(r, "name")?,
        "currency": text(r, "currency")?,
        "securityId": text(r, "security_id")?,
        "addedAt": text(r, "added_at")?,
    }))
}

pub fn list_watchlist(conn: &Connection) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM watchlist ORDER BY added_at, symbol")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(watch_from_row(r)?);
    }
    Ok(out)
}

/// `add_watch`: following a listing already followed keeps its place and
/// fills in what was blank.
pub fn add_watch(
    conn: &Connection,
    symbol: &str,
    exchange: &str,
    name: &str,
    currency: &str,
    security_id: &str,
    now: &str,
) -> Result<Option<Value>> {
    let sym = up(symbol);
    let ex = up(exchange);
    if sym.is_empty() {
        return Ok(None);
    }
    conn.execute(
        "INSERT OR IGNORE INTO watchlist (symbol, exchange, name, currency, security_id, added_at) VALUES (?, ?, ?, ?, ?, ?)",
        rusqlite::params![sym, ex, name, currency.to_uppercase(), security_id, now],
    )?;
    conn.execute(
        "UPDATE watchlist SET name = CASE WHEN COALESCE(name, '') = '' THEN ? ELSE name END, \
         currency = CASE WHEN COALESCE(currency, '') = '' THEN ? ELSE currency END, \
         security_id = CASE WHEN COALESCE(security_id, '') = '' THEN ? ELSE security_id END WHERE symbol = ? AND exchange = ?",
        rusqlite::params![name, currency.to_uppercase(), security_id, sym, ex],
    )?;
    let mut stmt = conn.prepare("SELECT * FROM watchlist WHERE symbol = ? AND exchange = ?")?;
    let mut rows = stmt.query(rusqlite::params![sym, ex])?;
    match rows.next()? { Some(r) => Ok(Some(watch_from_row(r)?)), None => Ok(None) }
}

pub fn remove_watch(conn: &Connection, symbol: &str, exchange: &str) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM watchlist WHERE symbol = ? AND exchange = ?",
        rusqlite::params![up(symbol), up(exchange)],
    )?;
    Ok(n > 0)
}

// --------------------------------------------------------------------------
// news
// --------------------------------------------------------------------------

/// `news_key`.
pub fn news_key(symbol: &str, exchange: &str) -> String {
    format!("{}@{}", up(symbol), up(exchange))
}

/// `replace_news`: a listing's latest items, in place of what it had. Each
/// row is stored under the source it was read from (`via`: tmx, nasdaq,
/// yahoo, sa, gnews), `source` when it names none.
pub fn replace_news(conn: &Connection, symbol: &str, exchange: &str, source: &str, rows: &[Value], now: &str) -> Result<()> {
    crate::atomically(conn, || {
        let sym = up(symbol);
        let ex = up(exchange);
        conn.execute("DELETE FROM news WHERE symbol = ? AND exchange = ?", rusqlite::params![sym, ex])?;
        for r in rows {
            let id = field_s(r, "id");
            if id.is_empty() {
                continue;
            }
            let kind = { let k = field_s(r, "kind"); if k.is_empty() { "story".to_string() } else { k } };
            conn.execute(
                "INSERT OR REPLACE INTO news (id, symbol, exchange, source, headline, wire, url, published_at, fetched_at, kind, summary) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    id, sym, ex, { let v = field_s(r, "via"); if v.is_empty() { source.to_string() } else { v } }, field_s(r, "headline"), field_s(r, "source"),
                    field_s(r, "url"), field_s(r, "publishedAt"), now, kind, field_s(r, "summary"),
                ],
            )?;
        }
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)",
            rusqlite::params![format!("news_fetched:{}", news_key(&sym, &ex)), now],
        )?;
        Ok(())
    })
}

/// `news_for`: a listing's stored items, newest first.
pub fn news_for(conn: &Connection, symbol: &str, exchange: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM news WHERE symbol = ? AND exchange = ? ORDER BY published_at DESC, id")?;
    let rows = stmt.query_map(rusqlite::params![up(symbol), up(exchange)], crate::snapshot::news_from_row)?;
    rows.collect()
}

/// `news_ids`: the ids a listing's stored items carry, so a wire's new
/// items can be told from the ones it had.
pub fn news_ids(conn: &Connection, symbol: &str, exchange: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM news WHERE symbol = ? AND exchange = ?")?;
    let rows = stmt.query_map(rusqlite::params![up(symbol), up(exchange)], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// `has_wire_release`: whether a wire has carried a release for this
/// ticker under any venue and any form the book writes it.
pub fn has_wire_release(conn: &Connection, symbol: &str) -> Result<bool> {
    let sym = up(symbol);
    if sym.is_empty() {
        return Ok(false);
    }
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM news WHERE kind = 'release' AND (symbol = ? OR symbol LIKE ?) LIMIT 1)",
        rusqlite::params![sym, format!("{}.%", sym)],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

pub fn news_fetched_at(conn: &Connection) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare("SELECT key, value FROM meta WHERE key LIKE 'news_fetched:%'")?;
    let mut rows = stmt.query([])?;
    let mut out = Map::new();
    while let Some(r) = rows.next()? {
        let key: String = r.get(0)?;
        out.insert(key["news_fetched:".len()..].to_string(), json!(r.get::<_, Option<String>>(1)?.unwrap_or_default()));
    }
    Ok(out)
}

pub fn forget_news(conn: &Connection, symbol: &str, exchange: &str) -> Result<()> {
    crate::atomically(conn, || {
        conn.execute("DELETE FROM news WHERE symbol = ? AND exchange = ?", rusqlite::params![up(symbol), up(exchange)])?;
        let tail = format!(":{}", news_key(symbol, exchange));
        conn.execute(
            "DELETE FROM meta WHERE key = ? OR (key LIKE 'news_source_fetched:%' AND substr(key, -length(?)) = ?)",
            rusqlite::params![format!("news_fetched:{}", news_key(symbol, exchange)), tail, tail],
        )?;
        Ok(())
    })
}

/// `trim_news`: keep the newest `keep` items over every symbol.
pub fn trim_news(conn: &Connection, keep: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM news WHERE rowid NOT IN (SELECT rowid FROM news ORDER BY published_at DESC, id LIMIT ?)",
        [keep],
    )?;
    Ok(())
}

// --------------------------------------------------------------------------
// filings
// --------------------------------------------------------------------------

pub fn filing_key(symbol: &str) -> String { up(symbol) }

/// A column that may not exist on this table at all, so it reads as empty
/// when absent: the legacy filings columns are
/// gone from a table created under the current schema.
fn maybe(r: &Row, name: &str) -> String {
    match r.as_ref().column_index(name) {
        Ok(i) => r.get::<_, Option<String>>(i).ok().flatten().unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// `_filing_from_row`: the old single-source columns stand in when the
/// new ones are empty, so a row written before the schema changed still reads.
fn filing_from_row(r: &Row) -> Result<Value> {
    let or = |a: &str, b: &str| -> Result<String> {
        let x = maybe(r, a);
        Ok(if x.is_empty() { maybe(r, b) } else { x })
    };
    Ok(json!({
        "id": maybe(r, "id"),
        "source": maybe(r, "source"),
        "category": maybe(r, "category"),
        "profileNo": maybe(r, "profile_no"),
        "issuer": maybe(r, "issuer"),
        "type": or("type", "file")?,
        "title": maybe(r, "title"),
        "date": or("date", "submitted_at")?,
        "dateText": or("date_text", "submitted")?,
        "size": maybe(r, "size"),
        "url": maybe(r, "url"),
        "subject": maybe(r, "subject"),
        "summary": maybe(r, "summary"),
        "enrichedAt": maybe(r, "enriched_at"),
        // a missing value reads as "", this one included
        "enrichVersion": match r.get::<_, Option<i64>>("enrich_version")? { Some(v) => json!(v), None => json!("") },
        // read for good: a regulator's form, read from its own boxes
        "enrichFinal": match r.as_ref().column_index("enrich_final") {
            Ok(_) => r.get::<_, Option<i64>>("enrich_final")?.map(|v| v != 0).unwrap_or(false),
            Err(_) => false,
        },
        "fetchedAt": maybe(r, "fetched_at"),
    }))
}

pub fn filings_for(conn: &Connection, symbol: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM filings WHERE symbol = ? ORDER BY date DESC, id")?;
    let mut rows = stmt.query([filing_key(symbol)])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(filing_from_row(r)?);
    }
    Ok(out)
}

pub fn filings_all(conn: &Connection) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare("SELECT * FROM filings ORDER BY symbol, date DESC, id")?;
    let mut rows = stmt.query([])?;
    let mut out: Map<String, Value> = Map::new();
    while let Some(r) = rows.next()? {
        let sym: String = r.get("symbol")?;
        out.entry(sym).or_insert_with(|| Value::Array(vec![])).as_array_mut().unwrap().push(filing_from_row(r)?);
    }
    Ok(out)
}

pub fn filing(conn: &Connection, symbol: &str, doc_id: &str) -> Result<Option<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM filings WHERE symbol = ? AND id = ?")?;
    let mut rows = stmt.query(rusqlite::params![filing_key(symbol), doc_id])?;
    match rows.next()? { Some(r) => Ok(Some(filing_from_row(r)?)), None => Ok(None) }
}

/// `set_filing_enrichment`: what was read out of a document, stamped
/// with the version of the logic that read it so a better one re-reads it
/// once. A value not given is left as it was.
pub fn set_filing_enrichment(
    conn: &Connection,
    symbol: &str,
    doc_id: &str,
    subject: Option<&str>,
    summary: Option<&str>,
    version: Option<i64>,
    final_: Option<bool>,
    now: &str,
) -> Result<()> {
    let mut sets: Vec<&str> = vec!["enriched_at = ?"];
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now.to_string())];
    // `final` marks a document read for good, which is not read again for a
    // half it will never have
    if let Some(f) = final_ {
        sets.push("enrich_final = ?");
        args.push(Box::new(if f { 1i64 } else { 0 }));
    }
    if let Some(s) = subject {
        sets.push("subject = ?");
        args.push(Box::new(s.to_string()));
    }
    if let Some(s) = summary {
        sets.push("summary = ?");
        args.push(Box::new(s.to_string()));
    }
    if let Some(v) = version {
        sets.push("enrich_version = ?");
        args.push(Box::new(v));
    }
    args.push(Box::new(filing_key(symbol)));
    args.push(Box::new(doc_id.to_string()));
    let sql = format!("UPDATE filings SET {} WHERE symbol = ? AND id = ?", sets.join(", "));
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    conn.execute(&sql, refs.as_slice())?;
    Ok(())
}

/// `replace_filings`: one source's disclosures for a symbol, in place of
/// what that source had.
///
/// Other sources' rows are untouched, and so is what has been read from the
/// documents: a row the source still lists keeps its subject and summary. The
/// list is refreshed far more often than a filed document changes, and
/// throwing the reading away with it meant every document was read again from
/// nothing on each refresh.
pub fn replace_filings(conn: &Connection, symbol: &str, source: &str, items: &[Value], now: &str) -> Result<usize> {
    crate::atomically(conn, || {
        let sym = filing_key(symbol);
        struct Kept { subject: Option<String>, summary: Option<String>, enriched_at: Option<String>, version: Option<i64>, final_: Option<i64> }
        let mut kept: Vec<(String, Kept)> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT id, subject, summary, enriched_at, enrich_version, enrich_final FROM filings WHERE symbol = ? AND source = ?",
            )?;
            let mut rows = stmt.query(rusqlite::params![sym, source])?;
            while let Some(r) = rows.next()? {
                kept.push((
                    r.get::<_, String>(0)?,
                    Kept { subject: r.get(1)?, summary: r.get(2)?, enriched_at: r.get(3)?, version: r.get(4)?, final_: r.get(5)? },
                ));
            }
        }
        conn.execute("DELETE FROM filings WHERE symbol = ? AND source = ?", rusqlite::params![sym, source])?;

        let mut n = 0usize;
        for r in items {
            let rid = field_s(r, "id");
            if rid.is_empty() {
                continue;
            }
            let src = if source.is_empty() { field_s(r, "source") } else { source.to_string() };
            let read = kept.iter().find(|(k, _)| *k == rid).map(|(_, v)| v);
            conn.execute(
                "INSERT OR REPLACE INTO filings (symbol, id, source, category, profile_no, issuer, type, title, date, date_text, size, url, fetched_at, \
                 subject, summary, enriched_at, enrich_version, enrich_final) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    sym, rid, src, field_s(r, "category"), field_s(r, "profileNo"), field_s(r, "issuer"),
                    field_s(r, "type"), field_s(r, "title"), field_s(r, "date"), field_s(r, "dateText"),
                    field_s(r, "size"), field_s(r, "url"), now,
                    read.map(|k| k.subject.clone().unwrap_or_default()).unwrap_or_default(),
                    read.map(|k| k.summary.clone().unwrap_or_default()).unwrap_or_default(),
                    read.and_then(|k| k.enriched_at.clone()),
                    read.and_then(|k| k.version),
                    read.and_then(|k| k.final_),
                ],
            )?;
            n += 1;
        }
        Ok(n)
    })
}

// --------------------------------------------------------------------------
// short selling
// --------------------------------------------------------------------------

/// `SHORT_FIELDS` and `_SHORT_COLUMNS`, paired.
pub const SHORT_FIELDS: [(&str, &str); 16] = [
    ("market", "market"), ("asOf", "as_of"), ("shares", "shares"), ("previous", "previous"),
    ("previousOf", "previous_of"), ("change", "change"), ("float", "float_shares"), ("ofFloat", "of_float"),
    ("averageVolume", "average_volume"), ("daysToCover", "days_to_cover"), ("volumeOf", "volume_of"),
    ("volumeSpan", "volume_span"), ("shortVolume", "short_volume"), ("totalVolume", "total_volume"),
    ("volumePct", "volume_pct"), ("name", "name"),
];

/// `_short_row`.
pub fn short_row(r: &Row) -> Result<Value> {
    let mut out = Map::new();
    out.insert("symbol".into(), json!(text(r, "symbol")?));
    out.insert("exchange".into(), json!(text(r, "exchange")?));
    out.insert("fetchedAt".into(), json!(text(r, "fetched_at")?));
    out.insert("readVersion".into(), json!(r.get::<_, Option<i64>>("read_version")?.unwrap_or(0)));
    for (name, column) in SHORT_FIELDS {
        let v = match r.get_ref(r.as_ref().column_index(column)?)? {
            rusqlite::types::ValueRef::Null => Value::Null,
            rusqlite::types::ValueRef::Integer(n) => json!(n),
            rusqlite::types::ValueRef::Real(f) => json!(f),
            rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t).to_string()),
            rusqlite::types::ValueRef::Blob(_) => Value::Null,
        };
        out.insert(name.into(), v);
    }
    let series: Option<String> = r.get("series")?;
    out.insert(
        "series".into(),
        match series {
            Some(s) if !s.is_empty() => serde_json::from_str(&s).unwrap_or_else(|_| json!([])),
            _ => json!([]),
        },
    );
    Ok(Value::Object(out))
}

/// `save_shorts`: one listing's short selling.
///
/// A run of reports already stored is not dropped by a later read that did not
/// ask for one.
pub fn save_shorts(conn: &Connection, symbol: &str, exchange: &str, rec: &Value, now: &str, version: i64) -> Result<()> {
    crate::atomically(conn, || {
        let sym = up(symbol);
        let ex = up(exchange);
        let series = match rec.get("series") {
            Some(s) if !s.is_null() => s.clone(),
            _ => {
                let held: Option<String> = conn
                    .query_row(
                        "SELECT series FROM shorts WHERE symbol = ? AND exchange = ?",
                        rusqlite::params![sym, ex],
                        |r| r.get(0),
                    )
                    .unwrap_or(None);
                match held {
                    Some(h) if !h.is_empty() => serde_json::from_str(&h).unwrap_or_else(|_| json!([])),
                    _ => json!([]),
                }
            }
        };
        let cols: Vec<&str> = SHORT_FIELDS.iter().map(|(_, c)| *c).collect();
        let marks = vec!["?"; cols.len() + 5].join(", ");
        let sql = format!(
            "INSERT OR REPLACE INTO shorts (symbol, exchange, {}, series, read_version, fetched_at) VALUES ({})",
            cols.join(", "),
            marks
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(sym.clone()), Box::new(ex.clone())];
        for (field, _) in SHORT_FIELDS {
            args.push(crate::activities::to_sql(rec.get(field).unwrap_or(&Value::Null)));
        }
        args.push(Box::new(crate::tables::json_text(&series)));
        args.push(Box::new(version));
        args.push(Box::new(now.to_string()));
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        conn.execute(&sql, refs.as_slice())?;
        Ok(())
    })
}

pub fn shorts_for(conn: &Connection, symbol: &str, exchange: &str) -> Result<Option<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM shorts WHERE symbol = ? AND exchange = ?")?;
    let mut rows = stmt.query(rusqlite::params![up(symbol), up(exchange)])?;
    match rows.next()? { Some(r) => Ok(Some(short_row(r)?)), None => Ok(None) }
}

// --------------------------------------------------------------------------
// gauges
// --------------------------------------------------------------------------

/// `save_gauge`: one published index's reading. What the publisher gives
/// beyond the score travels in the payload.
pub fn save_gauge(conn: &Connection, name: &str, rec: &Value, now: &str, version: i64) -> Result<()> {
    crate::atomically(conn, || {
        let key = name.trim().to_lowercase();
        let mut rest = Map::new();
        if let Some(m) = rec.as_object() {
            for (k, v) in m {
                if !["index", "source", "score", "rating", "asOf"].contains(&k.as_str()) {
                    rest.insert(k.clone(), v.clone());
                }
            }
        }
        conn.execute(
            "INSERT OR REPLACE INTO gauges (name, source, score, rating, as_of, payload, read_version, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                key,
                field_s(rec, "source"),
                opt_num(get(rec, "score")),
                field_s(rec, "rating"),
                field_s(rec, "asOf"),
                crate::tables::json_text(&Value::Object(rest)),
                version,
                now,
            ],
        )?;
        Ok(())
    })
}

pub fn gauge(conn: &Connection, name: &str) -> Result<Option<Value>> {
    let key = name.trim().to_lowercase();
    let mut stmt = conn.prepare("SELECT * FROM gauges WHERE name = ?")?;
    let mut rows = stmt.query([key])?;
    let r = match rows.next()? { Some(r) => r, None => return Ok(None) };
    let payload: Option<String> = r.get("payload")?;
    let rest: Value = match payload {
        Some(p) if !p.is_empty() => serde_json::from_str(&p).unwrap_or_else(|_| json!({})),
        _ => json!({}),
    };
    let mut out = Map::new();
    out.insert("index".into(), json!(text(r, "name")?));
    out.insert("source".into(), json!(text(r, "source")?));
    out.insert("score".into(), real(r, "score")?);
    out.insert("rating".into(), json!(text(r, "rating")?));
    out.insert("asOf".into(), json!(text(r, "as_of")?));
    out.insert("fetchedAt".into(), json!(text(r, "fetched_at")?));
    out.insert("readVersion".into(), json!(r.get::<_, Option<i64>>("read_version")?.unwrap_or(0)));
    if let Some(m) = rest.as_object() {
        for (k, v) in m {
            out.insert(k.clone(), v.clone());
        }
    }
    Ok(Some(Value::Object(out)))
}

// --------------------------------------------------------------------------
// told: the streams' memory of what they have met
// --------------------------------------------------------------------------

/// `TOLD_KEPT_DAYS`: a year and a bit -- long enough that nothing recurs,
/// small enough to stay tidy.
pub const TOLD_KEPT_DAYS: i64 = 400;

/// `events_told`: which of these the stream has already met -- told or
/// absorbed as history.
///
/// An event is what the thing is (a release's headline, a filing's own
/// marks), never the id a source gave it, so the same event from another
/// source, under another id, on another date, is still the same event.
pub fn events_told(conn: &Connection, scope: &str, events: &[String]) -> Result<HashSet<String>> {
    let want: Vec<String> = events.iter().filter(|e| !e.is_empty()).cloned().collect();
    let mut out: HashSet<String> = HashSet::new();
    if want.is_empty() {
        return Ok(out);
    }
    for chunk in want.chunks(400) {
        let holes = std::iter::repeat("?").take(chunk.len()).collect::<Vec<_>>().join(",");
        let sql = format!("SELECT event FROM told WHERE scope = ? AND event IN ({})", holes);
        let mut stmt = conn.prepare(&sql)?;
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&scope];
        for e in chunk {
            args.push(e);
        }
        let rows = stmt.query_map(args.as_slice(), |r| r.get::<_, String>(0))?;
        for r in rows {
            out.insert(r?);
        }
    }
    Ok(out)
}

/// `mark_told`: record that the stream has met these, whether or not they
/// were worth telling about.
pub fn mark_told(conn: &Connection, scope: &str, events: &[String], now: &str) -> Result<usize> {
    crate::atomically(conn, || {
        let rows: Vec<&String> = events.iter().filter(|e| !e.is_empty()).collect();
        if rows.is_empty() {
            return Ok(0);
        }
        for e in &rows {
            conn.execute("INSERT OR IGNORE INTO told(scope, event, at) VALUES (?, ?, ?)", rusqlite::params![scope, e, now])?;
        }
        let cutoff = bagholder_model::clock::stamp_days_ago(TOLD_KEPT_DAYS);
        conn.execute("DELETE FROM told WHERE at < ?", [cutoff])?;
        Ok(rows.len())
    })
}

// --------------------------------------------------------------------------
// notifications
// --------------------------------------------------------------------------

fn notification(r: &Row) -> Result<Value> {
    let extra: Option<String> = r.get("extra")?;
    let extra: Value = match extra {
        Some(e) if !e.is_empty() => serde_json::from_str(&e).unwrap_or_else(|_| json!({})),
        _ => json!({}),
    };
    Ok(json!({
        "id": r.get::<_, i64>("id")?,
        "at": text(r, "at")?,
        "kind": text(r, "kind")?,
        "key": text(r, "key")?,
        "title": text(r, "title")?,
        "body": text(r, "body")?,
        "extra": extra,
        "seenAt": text(r, "seen_at")?,
        "readAt": text(r, "read_at")?,
    }))
}

/// `add_notification`: one row, keyed so the same event is never stored
/// twice; the oldest beyond the last kept go.
///
/// A row the server shows itself is stored seen, so no page shows it too.
/// Nothing is returned when the key is already there.
pub fn add_notification(
    conn: &Connection,
    kind: &str,
    key: &str,
    title: &str,
    body: &str,
    extra: Option<&Value>,
    seen: bool,
    now: &str,
) -> Result<Option<Value>> {
    let extra = extra.cloned().unwrap_or_else(|| json!({}));
    let n = conn.execute(
        "INSERT OR IGNORE INTO notifications(at, kind, key, title, body, extra, seen_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![now, kind, key, title, body, crate::tables::json_text(&extra), if seen { Some(now) } else { None }],
    )?;
    if n == 0 {
        return Ok(None);
    }
    let rid = conn.last_insert_rowid();
    conn.execute(
        "DELETE FROM notifications WHERE id <= (SELECT id FROM notifications ORDER BY id DESC LIMIT 1 OFFSET ?)",
        [NOTIFICATIONS_KEPT],
    )?;
    let mut stmt = conn.prepare("SELECT * FROM notifications WHERE id = ?")?;
    let mut rows = stmt.query([rid])?;
    match rows.next()? { Some(r) => Ok(Some(notification(r)?)), None => Ok(None) }
}

/// `list_notifications`: rows after an id, and from a time when one is
/// given, oldest first -- newest first for the history.
pub fn list_notifications(
    conn: &Connection,
    after_id: i64,
    since: &str,
    unseen: bool,
    limit: i64,
    newest: bool,
) -> Result<Vec<Value>> {
    let mut sql = String::from("SELECT * FROM notifications WHERE id > ?");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(after_id)];
    if !since.is_empty() {
        sql.push_str(" AND at >= ?");
        args.push(Box::new(since.to_string()));
    }
    if unseen {
        sql.push_str(" AND seen_at IS NULL");
    }
    sql.push_str(if newest { " ORDER BY id DESC LIMIT ?" } else { " ORDER BY id ASC LIMIT ?" });
    args.push(Box::new(limit));
    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let mut rows = stmt.query(refs.as_slice())?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(notification(r)?);
    }
    Ok(out)
}

/// `mark_notifications_seen`: a page has shown these, so no page shows
/// them again.
pub fn mark_notifications_seen(conn: &Connection, ids: &[i64], now: &str) -> Result<usize> {
    crate::atomically(conn, || {
        if ids.is_empty() {
            return Ok(0);
        }
        let marks = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("UPDATE notifications SET seen_at = ? WHERE seen_at IS NULL AND id IN ({})", marks);
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now.to_string())];
        for i in ids {
            args.push(Box::new(*i));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        conn.execute(&sql, refs.as_slice())
    })
}

// --------------------------------------------------------------------------
// universes
// --------------------------------------------------------------------------

/// `replace_universe`.
pub fn replace_universe(conn: &Connection, key: &str, rows: &[Value], now: &str) -> Result<()> {
    crate::atomically(conn, || {
        conn.execute("DELETE FROM universes WHERE key = ?", [key])?;
        for r in rows {
            // the row is kept when its symbol is present and non-empty
            if !r.get("symbol").map(truthy).unwrap_or(false) {
                continue;
            }
            // the value and the change are stored as they came, not coerced
            let value = crate::activities::to_sql(r.get("value").unwrap_or(&Value::Null));
            let change = crate::activities::to_sql(r.get("percentChange").unwrap_or(&Value::Null));
            conn.execute(
                "INSERT OR REPLACE INTO universes (key, symbol, name, value, percent_change, sector, country, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    key, field_s(r, "symbol"), field_s(r, "name"), value.as_ref(), change.as_ref(),
                    field_s(r, "sector"), field_s(r, "country"), now,
                ],
            )?;
        }
        Ok(())
    })
}

// --------------------------------------------------------------------------
// the remaining readers
// --------------------------------------------------------------------------

/// `dividend_symbols`: the symbols that have paid, with the listing
/// exchange when the securities table knows it.
pub fn dividend_symbols(conn: &Connection) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT a.symbol AS symbol, a.currency AS currency, s.primary_exchange AS exchange \
         FROM activities a LEFT JOIN securities s ON s.id = a.security_id \
         WHERE a.category = 'dividend' AND IFNULL(a.symbol, '') != ''",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    while let Some(r) = rows.next()? {
        let sym = up(&text(r, "symbol")?);
        if sym.is_empty() || seen.contains(&sym) {
            continue;
        }
        seen.push(sym.clone());
        out.push(json!({
            "symbol": sym,
            "currency": text(r, "currency")?,
            "exchange": text(r, "exchange")?.trim().to_string(),
        }));
    }
    Ok(out)
}

/// `all_shorts`: every listing's stored short selling, for the ranked
/// list.
pub fn all_shorts(conn: &Connection) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT * FROM shorts")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(short_row(r)?);
    }
    Ok(out)
}

/// `mark_filings_fetched`: when a symbol's disclosures were last
/// refreshed, and its SEDAR+ profile number when one was found.
pub fn mark_filings_fetched(conn: &Connection, symbol: &str, profile_no: &str, now: &str) -> Result<()> {
    crate::atomically(conn, || {
        let sym = filing_key(symbol);
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)",
            rusqlite::params![format!("filings_fetched:{}", sym), now],
        )?;
        if !profile_no.is_empty() {
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)",
                rusqlite::params![format!("sedar_profile:{}", sym), profile_no],
            )?;
        }
        Ok(())
    })
}

pub fn filings_fetched_for(conn: &Connection, symbol: &str) -> Result<String> {
    Ok(crate::tables::get_meta(conn, &format!("filings_fetched:{}", filing_key(symbol)), "")?)
}

pub fn filings_fetched_at(conn: &Connection) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare("SELECT key, value FROM meta WHERE key LIKE 'filings_fetched:%'")?;
    let mut rows = stmt.query([])?;
    let mut out = Map::new();
    while let Some(r) = rows.next()? {
        let key: String = r.get(0)?;
        out.insert(
            key["filings_fetched:".len()..].to_string(),
            json!(r.get::<_, Option<String>>(1)?.unwrap_or_default()),
        );
    }
    Ok(out)
}

/// `sedar_profile`.
pub fn sedar_profile(conn: &Connection, symbol: &str) -> Result<String> {
    crate::tables::get_meta(conn, &format!("sedar_profile:{}", filing_key(symbol)), "")
}

pub fn forget_filings(conn: &Connection, symbol: &str) -> Result<()> {
    crate::atomically(conn, || {
        let sym = filing_key(symbol);
        conn.execute("DELETE FROM filings WHERE symbol = ?", [&sym])?;
        conn.execute(
            "DELETE FROM meta WHERE key IN (?, ?)",
            rusqlite::params![format!("filings_fetched:{}", sym), format!("sedar_profile:{}", sym)],
        )?;
        Ok(())
    })
}

/// `sold_since`: the shares sold in an account since a moment, from the
/// activity feed -- by security id when there is one, else by symbol.
pub fn sold_since(conn: &Connection, account_id: &str, security_id: &str, since_iso: &str, symbol: &str) -> Result<f64> {
    let q: Option<f64> = if !security_id.is_empty() {
        conn.query_row(
            "SELECT SUM(quantity) FROM activities WHERE account_id = ? AND security_id = ? AND activity_type = 'Trade' AND activity_sub_type = 'SELL' AND occurred_at > ?",
            rusqlite::params![account_id, security_id, since_iso],
            |r| r.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT SUM(quantity) FROM activities WHERE account_id = ? AND symbol = ? AND activity_type = 'Trade' AND activity_sub_type = 'SELL' AND occurred_at > ?",
            rusqlite::params![account_id, symbol, since_iso],
            |r| r.get(0),
        )?
    };
    Ok(q.unwrap_or(0.0))
}

/// `position_quantity`: Wealthsimple's own balance for one security in
/// one account, as last read. Nothing when it is not known.
pub fn position_quantity(conn: &Connection, account_id: &str, security_id: &str) -> Result<Option<f64>> {
    conn.query_row(
        "SELECT SUM(quantity) FROM balances WHERE account_id = ? AND security_id = ?",
        rusqlite::params![account_id, security_id],
        |r| r.get(0),
    )
}

pub fn balances_count(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM balances", [], |r| r.get(0))
}

pub fn latest_notification_id(conn: &Connection) -> Result<i64> {
    let m: Option<i64> = conn.query_row("SELECT MAX(id) FROM notifications", [], |r| r.get(0))?;
    Ok(m.unwrap_or(0))
}

/// `unread_notifications`: how many the person has not looked at.
pub fn unread_notifications(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM notifications WHERE read_at IS NULL", [], |r| r.get(0))
}

/// `mark_notifications_read`: every unread one when no ids are given.
pub fn mark_notifications_read(conn: &Connection, ids: Option<&[i64]>, now: &str) -> Result<usize> {
    crate::atomically(conn, || {
        match ids {
            None => conn.execute("UPDATE notifications SET read_at = ? WHERE read_at IS NULL", [now]),
            Some(ids) if ids.is_empty() => Ok(0),
            Some(ids) => {
                let marks = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!("UPDATE notifications SET read_at = ? WHERE read_at IS NULL AND id IN ({})", marks);
                let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now.to_string())];
                for i in ids {
                    args.push(Box::new(*i));
                }
                let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
                conn.execute(&sql, refs.as_slice())
            }
        }
    })
}

/// `clear_notifications`: the history emptied, and the keys with it --
/// an event already told is only untold while its row stands.
pub fn clear_notifications(conn: &Connection) -> Result<usize> {
    conn.execute("DELETE FROM notifications", [])
}

/// `exposure_record`: one record by its key, or nothing.
pub fn exposure_record(conn: &Connection, key: &str) -> Result<Option<Value>> {
    let all = crate::admin::exposures_map(conn)?;
    Ok(all.get(key).cloned())
}
