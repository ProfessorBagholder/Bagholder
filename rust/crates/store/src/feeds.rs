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

use bagholder_model::value::FSum;

/// `NOTIFICATIONS_KEPT`.
pub const NOTIFICATIONS_KEPT: i64 = 200;

fn text(r: &Row, name: &str) -> Result<String> {
    Ok(r.get::<_, Option<String>>(name)?.unwrap_or_default())
}

fn up(s: &str) -> String { s.trim().to_uppercase() }

// --------------------------------------------------------------------------
// exposure
// --------------------------------------------------------------------------

/// Names and their weights, in the order the source gave them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Weights(pub Vec<(String, f64)>);

impl Weights {
    /// Adds to an existing name's weight, else appends.
    pub fn add(&mut self, name: &str, w: f64) {
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 += w,
            None => self.0.push((name.to_string(), w)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, (String, f64)> {
        self.0.iter()
    }

    pub fn first_name(&self) -> Option<&str> {
        self.0.first().map(|(n, _)| n.as_str())
    }

    pub fn total(&self) -> f64 {
        self.0.iter().map(|(_, w)| *w).fsum()
    }

    /// Each weight divided by `by`.
    pub fn scaled(self, by: f64) -> Weights {
        Weights(self.0.into_iter().map(|(n, w)| (n, w / by)).collect())
    }
}

impl serde::Serialize for Weights {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (n, w) in &self.0 {
            m.serialize_entry(n, w)?;
        }
        m.end()
    }
}

impl<'de> serde::Deserialize<'de> for Weights {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Weights, D::Error> {
        let v = Value::deserialize(d)?;
        Ok(match v {
            Value::Object(m) => Weights(m.into_iter().map(|(k, v)| (k, bagholder_model::value::num(Some(&v), 0.0))).collect()),
            _ => Weights::default(),
        })
    }
}

/// One security's or listing's exposure: sector and country weights as
/// fractions, the share of the holding they cover, and where they came from.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureRecord {
    pub sectors: Weights,
    pub countries: Weights,
    pub coverage: f64,
    pub source: String,
    pub as_of: String,
    pub industry: String,
    pub error: String,
}

/// An exposure record as stored: when it was read.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredExposure {
    #[serde(flatten)]
    pub record: ExposureRecord,
    pub fetched_at: String,
}

/// One row of the `exposures` table into a `StoredExposure`, for
/// `exposure_record` and `snapshot::exposures_part`.
pub fn stored_exposure(r: &Row) -> Result<StoredExposure> {
    let weights = |raw: Option<String>| -> Weights {
        match raw {
            Some(s) if !s.is_empty() => serde_json::from_str(&s).unwrap_or_default(),
            _ => Weights::default(),
        }
    };
    Ok(StoredExposure {
        record: ExposureRecord {
            sectors: weights(r.get("sectors")?),
            countries: weights(r.get("countries")?),
            coverage: r.get::<_, Option<f64>>("coverage")?.unwrap_or(0.0),
            source: text(r, "source")?,
            as_of: text(r, "as_of")?,
            industry: text(r, "industry")?,
            error: text(r, "error")?,
        },
        fetched_at: text(r, "fetched_at")?,
    })
}

/// `replace_exposure`: one record -- the sectors and countries as
/// `{name: fraction}`, the share of the holding they cover, and where it came
/// from.
pub fn replace_exposure(conn: &Connection, key: &str, rec: &ExposureRecord, now: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO exposures (key, sectors, countries, coverage, source, as_of, industry, error, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(key) DO UPDATE SET sectors = excluded.sectors, countries = excluded.countries, coverage = excluded.coverage, source = excluded.source, \
         as_of = excluded.as_of, industry = excluded.industry, error = excluded.error, fetched_at = excluded.fetched_at",
        rusqlite::params![
            key,
            serde_json::to_string(&rec.sectors).unwrap_or_default(),
            serde_json::to_string(&rec.countries).unwrap_or_default(),
            rec.coverage,
            rec.source,
            rec.as_of,
            rec.industry,
            rec.error,
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

/// The feed a news item was read from. Declared in the order a wire's feeds
/// stand in for each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Feed {
    Tmx,
    TmxMedia,
    Nasdaq,
    NasdaqPress,
    Yahoo,
    Sa,
    Gnews,
}

impl Feed {
    pub fn as_str(self) -> &'static str {
        match self {
            Feed::Tmx => "tmx",
            Feed::TmxMedia => "tmx-media",
            Feed::Nasdaq => "nasdaq",
            Feed::NasdaqPress => "nasdaq-press",
            Feed::Yahoo => "yahoo",
            Feed::Sa => "sa",
            Feed::Gnews => "gnews",
        }
    }

    pub fn parse(s: &str) -> Option<Feed> {
        match s {
            "tmx" => Some(Feed::Tmx),
            "tmx-media" => Some(Feed::TmxMedia),
            "nasdaq" => Some(Feed::Nasdaq),
            "nasdaq-press" => Some(Feed::NasdaqPress),
            "yahoo" => Some(Feed::Yahoo),
            "sa" => Some(Feed::Sa),
            "gnews" => Some(Feed::Gnews),
            _ => None,
        }
    }

    /// The source an item's id names by its prefix (`tmx:`, `nasdaq:`,
    /// `yahoo:`, `sa:`, `gnews:`).
    pub fn of_id(id: &str) -> Option<Feed> {
        match id.split_once(':').map(|(p, _)| p) {
            Some("tmx") => Some(Feed::Tmx),
            Some("nasdaq") => Some(Feed::Nasdaq),
            Some("yahoo") => Some(Feed::Yahoo),
            Some("sa") => Some(Feed::Sa),
            Some("gnews") => Some(Feed::Gnews),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NewsKind {
    #[default]
    Story,
    Release,
}

impl NewsKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NewsKind::Story => "story",
            NewsKind::Release => "release",
        }
    }

    pub fn parse(s: &str) -> NewsKind {
        if s == "release" {
            NewsKind::Release
        } else {
            NewsKind::Story
        }
    }
}

/// An item as a source answers it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    pub id: String,
    pub headline: String,
    /// The publisher or wire that carried it.
    pub source: String,
    pub url: String,
    pub published_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary: String,
    pub kind: NewsKind,
    pub via: Feed,
}

/// A stored item, as read back for a listing.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredNews {
    pub id: String,
    pub symbol: String,
    pub exchange: String,
    /// The feed it was read from; `None` for a stored value that names none.
    #[serde(rename = "source")]
    pub feed: Option<Feed>,
    pub headline: String,
    /// The publisher or wire that carried it.
    pub wire: String,
    pub url: String,
    pub published_at: String,
    pub fetched_at: String,
    pub kind: NewsKind,
    pub summary: String,
}

impl StoredNews {
    /// The item again, as its feed answered it; `None` when no feed is named.
    pub fn item(&self) -> Option<NewsItem> {
        Some(NewsItem {
            id: self.id.clone(),
            headline: self.headline.clone(),
            source: self.wire.clone(),
            url: self.url.clone(),
            published_at: self.published_at.clone(),
            summary: self.summary.clone(),
            kind: self.kind,
            via: self.feed?,
        })
    }
}

/// `news_key`.
pub fn news_key(symbol: &str, exchange: &str) -> String {
    format!("{}@{}", up(symbol), up(exchange))
}

/// `replace_news`: a listing's latest items, in place of what it had. Each
/// row is stored under the feed it was read from (`via`).
pub fn replace_news(conn: &Connection, symbol: &str, exchange: &str, rows: &[NewsItem], now: &str) -> Result<()> {
    crate::atomically(conn, || {
        let sym = up(symbol);
        let ex = up(exchange);
        let changed = crate::gens::replace_if_changed(conn, "SELECT id, source, headline, wire, url, published_at, kind, summary FROM news WHERE symbol = ? AND exchange = ?", rusqlite::params![sym, ex], || {
        conn.execute("DELETE FROM news WHERE symbol = ? AND exchange = ?", rusqlite::params![sym, ex])?;
        for r in rows {
            if r.id.is_empty() {
                continue;
            }
            conn.execute(
                "INSERT OR REPLACE INTO news (id, symbol, exchange, source, headline, wire, url, published_at, fetched_at, kind, summary) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    r.id, sym, ex, r.via.as_str(), r.headline, r.source,
                    r.url, r.published_at, now, r.kind.as_str(), r.summary,
                ],
            )?;
        }
        Ok(())
        })?;
        if !changed {
            conn.execute("UPDATE news SET fetched_at = ? WHERE symbol = ? AND exchange = ?", rusqlite::params![now, sym, ex])?;
        }
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)",
            rusqlite::params![format!("news_fetched:{}", news_key(&sym, &ex)), now],
        )?;
        Ok(())
    })
}

/// `_news_from_row`: an item with no kind is a story, which is what a
/// row stored before releases were told apart is.
fn news_from_row(r: &Row) -> Result<StoredNews> {
    let kind = NewsKind::parse(&text(r, "kind")?);
    let id = text(r, "id")?;
    let source = text(r, "source")?;
    let feed = Feed::parse(&source).or_else(|| Feed::of_id(&id));
    Ok(StoredNews {
        id,
        symbol: text(r, "symbol")?,
        exchange: text(r, "exchange")?,
        feed,
        headline: text(r, "headline")?,
        wire: text(r, "wire")?,
        url: text(r, "url")?,
        published_at: text(r, "published_at")?,
        fetched_at: text(r, "fetched_at")?,
        kind,
        summary: text(r, "summary")?,
    })
}

/// `news_for`: a listing's stored items, newest first.
pub fn news_for(conn: &Connection, symbol: &str, exchange: &str) -> Result<Vec<StoredNews>> {
    let mut stmt = conn.prepare("SELECT * FROM news WHERE symbol = ? AND exchange = ? ORDER BY published_at DESC, id")?;
    let rows = stmt.query_map(rusqlite::params![up(symbol), up(exchange)], news_from_row)?;
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

/// The regulator a document was filed with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub enum Regulator {
    #[serde(rename = "SEDAR+")]
    Sedar,
    #[serde(rename = "SEC")]
    Sec,
}

impl Regulator {
    pub fn as_str(self) -> &'static str {
        match self {
            Regulator::Sedar => "SEDAR+",
            Regulator::Sec => "SEC",
        }
    }

    pub fn parse(s: &str) -> Option<Regulator> {
        match s {
            "SEDAR+" => Some(Regulator::Sedar),
            "SEC" => Some(Regulator::Sec),
            _ => None,
        }
    }
}

/// A document a regulator lists for an issuer.
#[derive(Clone, Debug, PartialEq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct FiledDocument {
    pub id: String,
    pub source: Regulator,
    pub category: String,
    pub profile_no: String,
    pub issuer: String,
    /// The form or document type.
    #[serde(rename = "type")]
    pub form: String,
    pub title: String,
    pub date: String,
    pub date_text: String,
    pub size: String,
    pub url: String,
}

/// A filed document as stored, with what a reading made of it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct Filing {
    #[serde(flatten)]
    #[ts(flatten)]
    pub doc: FiledDocument,
    pub subject: String,
    pub summary: String,
    pub enriched_at: String,
    /// The version of the logic that read it; `None` when never read.
    pub enrich_version: Option<i64>,
    /// Read for good: a form read from its own boxes is not read again.
    pub enrich_final: bool,
    pub fetched_at: String,
}

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
/// A row whose source names no known regulator is skipped: only SEDAR+ and
/// SEC exist.
fn filing_from_row(r: &Row) -> Result<Option<Filing>> {
    let or = |a: &str, b: &str| -> String {
        let x = maybe(r, a);
        if x.is_empty() { maybe(r, b) } else { x }
    };
    let source = match Regulator::parse(&maybe(r, "source")) {
        Some(s) => s,
        None => return Ok(None),
    };
    let doc = FiledDocument {
        id: maybe(r, "id"),
        source,
        category: maybe(r, "category"),
        profile_no: maybe(r, "profile_no"),
        issuer: maybe(r, "issuer"),
        form: or("type", "file"),
        title: maybe(r, "title"),
        date: or("date", "submitted_at"),
        date_text: or("date_text", "submitted"),
        size: maybe(r, "size"),
        url: maybe(r, "url"),
    };
    Ok(Some(Filing {
        doc,
        subject: maybe(r, "subject"),
        summary: maybe(r, "summary"),
        enriched_at: maybe(r, "enriched_at"),
        enrich_version: r.get::<_, Option<i64>>("enrich_version")?,
        // read for good: a regulator's form, read from its own boxes
        enrich_final: match r.as_ref().column_index("enrich_final") {
            Ok(_) => r.get::<_, Option<i64>>("enrich_final")?.map(|v| v != 0).unwrap_or(false),
            Err(_) => false,
        },
        fetched_at: maybe(r, "fetched_at"),
    }))
}

pub fn filings_for(conn: &Connection, symbol: &str) -> Result<Vec<Filing>> {
    let mut stmt = conn.prepare("SELECT * FROM filings WHERE symbol = ? ORDER BY date DESC, id")?;
    let mut rows = stmt.query([filing_key(symbol)])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        if let Some(fl) = filing_from_row(r)? {
            out.push(fl);
        }
    }
    Ok(out)
}

pub fn filings_all(conn: &Connection) -> Result<std::collections::BTreeMap<String, Vec<Filing>>> {
    let mut stmt = conn.prepare("SELECT * FROM filings ORDER BY symbol, date DESC, id")?;
    let mut rows = stmt.query([])?;
    let mut out: std::collections::BTreeMap<String, Vec<Filing>> = std::collections::BTreeMap::new();
    while let Some(r) = rows.next()? {
        let sym: String = r.get("symbol")?;
        if let Some(fl) = filing_from_row(r)? {
            out.entry(sym).or_default().push(fl);
        }
    }
    Ok(out)
}

pub fn filing(conn: &Connection, symbol: &str, doc_id: &str) -> Result<Option<Filing>> {
    let mut stmt = conn.prepare("SELECT * FROM filings WHERE symbol = ? AND id = ?")?;
    let mut rows = stmt.query(rusqlite::params![filing_key(symbol), doc_id])?;
    match rows.next()? { Some(r) => filing_from_row(r), None => Ok(None) }
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
pub fn replace_filings(conn: &Connection, symbol: &str, source: Regulator, items: &[FiledDocument], now: &str) -> Result<usize> {
    crate::atomically(conn, || {
        let sym = filing_key(symbol);
        struct Kept { subject: Option<String>, summary: Option<String>, enriched_at: Option<String>, version: Option<i64>, final_: Option<i64> }
        let mut kept: Vec<(String, Kept)> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT id, subject, summary, enriched_at, enrich_version, enrich_final FROM filings WHERE symbol = ? AND source = ?",
            )?;
            let mut rows = stmt.query(rusqlite::params![sym, source.as_str()])?;
            while let Some(r) = rows.next()? {
                kept.push((
                    r.get::<_, String>(0)?,
                    Kept { subject: r.get(1)?, summary: r.get(2)?, enriched_at: r.get(3)?, version: r.get(4)?, final_: r.get(5)? },
                ));
            }
        }
        conn.execute("DELETE FROM filings WHERE symbol = ? AND source = ?", rusqlite::params![sym, source.as_str()])?;

        let mut n = 0usize;
        for it in items {
            if it.id.is_empty() {
                continue;
            }
            let read = kept.iter().find(|(k, _)| *k == it.id).map(|(_, v)| v);
            conn.execute(
                "INSERT OR REPLACE INTO filings (symbol, id, source, category, profile_no, issuer, type, title, date, date_text, size, url, fetched_at, \
                 subject, summary, enriched_at, enrich_version, enrich_final) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    sym, it.id, source.as_str(), it.category, it.profile_no, it.issuer,
                    it.form, it.title, it.date, it.date_text,
                    it.size, it.url, now,
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

/// "us" or "ca": the regulator's own market for a listing's short selling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
pub enum ShortMarket {
    #[default]
    Us,
    Ca,
}

impl ShortMarket {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShortMarket::Us => "us",
            ShortMarket::Ca => "ca",
        }
    }
}

/// Whether a short volume report covers one trading day (the US) or a
/// half-month period (Canada).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
pub enum VolumeSpan {
    Day,
    Period,
}

impl VolumeSpan {
    pub fn as_str(&self) -> &'static str {
        match self {
            VolumeSpan::Day => "day",
            VolumeSpan::Period => "period",
        }
    }
}

/// One reporting date's short position, as the run behind a listing's current
/// figure.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub struct ShortPoint {
    pub date: String,
    pub shares: f64,
}

/// One listing's short selling as its regulator publishes it: the position
/// still sold short and the short part of its recent trading. Neither
/// measure is estimated -- every figure here is the regulator's own, or
/// `daysToCover`, the one number the app derives from them.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct Shorts {
    pub symbol: String,
    pub exchange: String,
    pub market: ShortMarket,
    pub name: String,
    pub as_of: String,
    pub shares: Option<f64>,
    pub previous: Option<f64>,
    pub previous_of: String,
    pub change: Option<f64>,
    pub float: Option<f64>,
    pub of_float: Option<f64>,
    pub average_volume: Option<f64>,
    pub days_to_cover: Option<f64>,
    pub volume_of: String,
    pub volume_span: Option<VolumeSpan>,
    pub short_volume: Option<f64>,
    pub total_volume: Option<f64>,
    pub volume_pct: Option<f64>,
    /// The reports behind the position, oldest first; `None` where they were
    /// not read.
    pub series: Option<Vec<ShortPoint>>,
}

impl Default for Shorts {
    fn default() -> Self {
        Shorts {
            symbol: String::new(),
            exchange: String::new(),
            market: ShortMarket::default(),
            name: String::new(),
            as_of: String::new(),
            shares: None,
            previous: None,
            previous_of: String::new(),
            change: None,
            float: None,
            of_float: None,
            average_volume: None,
            days_to_cover: None,
            volume_of: String::new(),
            volume_span: None,
            short_volume: None,
            total_volume: None,
            volume_pct: None,
            series: None,
        }
    }
}

/// A listing's short selling as stored: when it was read, and by which
/// version of the reading.
#[derive(Clone, Debug, PartialEq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct StoredShorts {
    #[serde(flatten)]
    pub shorts: Shorts,
    pub fetched_at: String,
    pub read_version: i64,
}

/// `short_row`: `None` where the stored row's market is neither `us` nor
/// `ca` -- an older row from before the market was typed.
fn short_row(r: &Row) -> Result<Option<StoredShorts>> {
    let market = match r.get::<_, Option<String>>("market")?.as_deref() {
        Some("us") => ShortMarket::Us,
        Some("ca") => ShortMarket::Ca,
        _ => return Ok(None),
    };
    let volume_span = match r.get::<_, Option<String>>("volume_span")?.as_deref() {
        Some("day") => Some(VolumeSpan::Day),
        Some("period") => Some(VolumeSpan::Period),
        _ => None,
    };
    let series: Option<String> = r.get("series")?;
    let series: Vec<ShortPoint> = match series {
        Some(s) if !s.is_empty() => serde_json::from_str(&s).unwrap_or_default(),
        _ => vec![],
    };
    Ok(Some(StoredShorts {
        shorts: Shorts {
            symbol: text(r, "symbol")?,
            exchange: text(r, "exchange")?,
            market,
            name: text(r, "name")?,
            as_of: text(r, "as_of")?,
            shares: r.get("shares")?,
            previous: r.get("previous")?,
            previous_of: text(r, "previous_of")?,
            change: r.get("change")?,
            float: r.get("float_shares")?,
            of_float: r.get("of_float")?,
            average_volume: r.get("average_volume")?,
            days_to_cover: r.get("days_to_cover")?,
            volume_of: text(r, "volume_of")?,
            volume_span,
            short_volume: r.get("short_volume")?,
            total_volume: r.get("total_volume")?,
            volume_pct: r.get("volume_pct")?,
            // a stored run always reads back, empty where none was kept
            series: Some(series),
        },
        fetched_at: text(r, "fetched_at")?,
        read_version: r.get::<_, Option<i64>>("read_version")?.unwrap_or(0),
    }))
}

/// `save_shorts`: one listing's short selling.
///
/// A run of reports already stored is not dropped by a later read that did not
/// ask for one.
pub fn save_shorts(conn: &Connection, rec: &Shorts, now: &str, version: i64) -> Result<()> {
    crate::atomically(conn, || {
        let sym = up(&rec.symbol);
        let ex = up(&rec.exchange);
        let series: Vec<ShortPoint> = match &rec.series {
            Some(s) => s.clone(),
            None => {
                let held: Option<String> = conn
                    .query_row(
                        "SELECT series FROM shorts WHERE symbol = ? AND exchange = ?",
                        rusqlite::params![sym, ex],
                        |r| r.get(0),
                    )
                    .unwrap_or(None);
                match held {
                    Some(h) if !h.is_empty() => serde_json::from_str(&h).unwrap_or_default(),
                    _ => vec![],
                }
            }
        };
        conn.execute(
            "INSERT OR REPLACE INTO shorts (symbol, exchange, market, as_of, shares, previous, previous_of, change, \
             float_shares, of_float, average_volume, days_to_cover, volume_of, volume_span, short_volume, \
             total_volume, volume_pct, name, series, read_version, fetched_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                sym,
                ex,
                rec.market.as_str(),
                rec.as_of,
                rec.shares,
                rec.previous,
                rec.previous_of,
                rec.change,
                rec.float,
                rec.of_float,
                rec.average_volume,
                rec.days_to_cover,
                rec.volume_of,
                rec.volume_span.as_ref().map(|v| v.as_str()),
                rec.short_volume,
                rec.total_volume,
                rec.volume_pct,
                rec.name,
                serde_json::to_string(&series).unwrap_or_default(),
                version,
                now,
            ],
        )?;
        Ok(())
    })
}

/// One listing's stored short selling; `None` where none is stored, or where
/// the row stored is from before the market was typed.
pub fn shorts_for(conn: &Connection, symbol: &str, exchange: &str) -> Result<Option<StoredShorts>> {
    let mut stmt = conn.prepare("SELECT * FROM shorts WHERE symbol = ? AND exchange = ?")?;
    let mut rows = stmt.query(rusqlite::params![up(symbol), up(exchange)])?;
    match rows.next()? { Some(r) => short_row(r), None => Ok(None) }
}

// --------------------------------------------------------------------------
// gauges
// --------------------------------------------------------------------------

/// One published index's reading as its publisher gives it: the score now on
/// the publisher's own scale, the readings it compares itself against, its
/// indicators where it publishes them, and its daily history, oldest first.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct Gauge {
    pub index: String,
    pub source: String,
    pub score: f64,
    pub rating: String,
    pub as_of: String,
    pub previous: Vec<GaugeReading>,
    pub parts: Vec<GaugePart>,
    pub series: Vec<GaugePoint>,
}

/// An earlier reading the publisher compares the one now against.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub struct GaugeReading {
    pub label: String,
    pub score: f64,
    pub rating: String,
}

/// One of the indicators the publisher builds its score from.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub struct GaugePart {
    pub name: String,
    pub score: f64,
    pub rating: String,
}

/// One day's reading.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub struct GaugePoint {
    pub date: String,
    pub score: f64,
}

/// A reading as stored: when it was read, and by which version of the reading.
#[derive(Clone, Debug, PartialEq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct StoredGauge {
    #[serde(flatten)]
    pub gauge: Gauge,
    pub fetched_at: String,
    pub read_version: i64,
}

/// What a gauge's row keeps beyond its columns.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct GaugeRest {
    previous: Vec<GaugeReading>,
    parts: Vec<GaugePart>,
    series: Vec<GaugePoint>,
}

/// `save_gauge`: one published index's reading. What the publisher gives
/// beyond the score travels in the payload.
pub fn save_gauge(conn: &Connection, name: &str, rec: &Gauge, now: &str, version: i64) -> Result<()> {
    crate::atomically(conn, || {
        let key = name.trim().to_lowercase();
        let rest = GaugeRest { previous: rec.previous.clone(), parts: rec.parts.clone(), series: rec.series.clone() };
        conn.execute(
            "INSERT OR REPLACE INTO gauges (name, source, score, rating, as_of, payload, read_version, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![key, rec.source, rec.score, rec.rating, rec.as_of, serde_json::to_string(&rest).unwrap_or_default(), version, now],
        )?;
        Ok(())
    })
}

/// One index's stored reading; nothing where none is stored, or where the one
/// stored has no score.
pub fn gauge(conn: &Connection, name: &str) -> Result<Option<StoredGauge>> {
    let key = name.trim().to_lowercase();
    let mut stmt = conn.prepare("SELECT * FROM gauges WHERE name = ?")?;
    let mut rows = stmt.query([key])?;
    let r = match rows.next()? { Some(r) => r, None => return Ok(None) };
    let score: Option<f64> = r.get("score")?;
    let score = match score { Some(s) => s, None => return Ok(None) };
    let payload: Option<String> = r.get("payload")?;
    let rest: GaugeRest = payload.as_deref().and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default();
    Ok(Some(StoredGauge {
        gauge: Gauge {
            index: text(r, "name")?,
            source: text(r, "source")?,
            score,
            rating: text(r, "rating")?,
            as_of: text(r, "as_of")?,
            previous: rest.previous,
            parts: rest.parts,
            series: rest.series,
        },
        fetched_at: text(r, "fetched_at")?,
        read_version: r.get::<_, Option<i64>>("read_version")?.unwrap_or(0),
    }))
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
pub fn replace_universe(conn: &Connection, key: &str, rows: &[bagholder_model::input::UniverseRow], now: &str) -> Result<()> {
    crate::atomically(conn, || {
        let changed = crate::gens::replace_if_changed(conn, "SELECT symbol, name, value, percent_change, sector, country FROM universes WHERE key = ?", rusqlite::params![key], || {
        conn.execute("DELETE FROM universes WHERE key = ?", [key])?;
        for r in rows.iter().filter(|r| !r.symbol.is_empty()) {
            conn.execute(
                "INSERT OR REPLACE INTO universes (key, symbol, name, value, percent_change, sector, country, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![key, r.symbol, r.name, r.value, r.percent_change, r.sector, r.country, now],
            )?;
        }
        Ok(())
        })?;
        if !changed {
            conn.execute("UPDATE universes SET fetched_at = ? WHERE key = ?", rusqlite::params![now, key])?;
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
/// list. A row from before the market was typed is left out.
pub fn all_shorts(conn: &Connection) -> Result<Vec<StoredShorts>> {
    let mut stmt = conn.prepare("SELECT * FROM shorts")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        if let Some(row) = short_row(r)? {
            out.push(row);
        }
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
pub fn exposure_record(conn: &Connection, key: &str) -> Result<Option<StoredExposure>> {
    let mut stmt = conn.prepare("SELECT * FROM exposures WHERE key = ?")?;
    let mut rows = stmt.query(rusqlite::params![key])?;
    match rows.next()? { Some(r) => Ok(Some(stored_exposure(r)?)), None => Ok(None) }
}
