//! What the market readers' tables hold, read from the columns straight into the
//! model's own types: an exposure's weights, the news, the universes. A text that
//! is not there is empty, a number that must be one is zero, and one that may be
//! absent stays absent.

use rusqlite::{Connection, Result, Row};

use bagholder_model::exposure::{Exposure, Exposures};
use super::feeds::Weights;
use bagholder_model::input::{NewsRow, UniverseRow};
use bagholder_model::wire::Ordered;

fn text(r: &Row, name: &str) -> Result<String> {
    Ok(r.get::<_, Option<String>>(name)?.unwrap_or_default())
}

/// A number that may be absent. SQLite keeps no NaN, so a stored number is a number.
fn maybe(r: &Row, name: &str) -> Result<Option<f64>> {
    r.get::<_, Option<f64>>(name)
}

fn number(r: &Row, name: &str) -> Result<f64> {
    Ok(maybe(r, name)?.unwrap_or(0.0))
}

fn all<T>(conn: &Connection, sql: &str, f: impl Fn(&Row) -> Result<T>) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| f(r))?;
    rows.collect()
}

/// Weights kept as JSON text: an empty column is none, and a text that does not
/// parse is the row's failure (the store is corrupt there), never none.
fn weights(r: &Row, name: &str) -> Result<Weights> {
    let raw = text(r, name)?;
    if raw.is_empty() {
        return Ok(Weights::default());
    }
    let at = r.as_ref().column_index(name)?;
    serde_json::from_str(&raw).map_err(|e| rusqlite::Error::FromSqlConversionFailure(at, rusqlite::types::Type::Text, Box::new(e)))
}

/// Each security's weights, from the JSON text they are kept as.
pub fn exposures(conn: &Connection) -> Result<Exposures> {
    let rows = all(conn, "SELECT * FROM exposures", |r| {
        let sectors = weights(r, "sectors")?;
        let countries = weights(r, "countries")?;
        Ok((text(r, "key")?, Exposure { sectors: sectors.0, countries: countries.0 }))
    })?;
    Ok(rows.into_iter().collect())
}

/// An item with no kind is a story, which is what a row stored before releases
/// were told apart is.
pub fn news(conn: &Connection) -> Result<Vec<NewsRow>> {
    all(conn, "SELECT * FROM news ORDER BY published_at DESC, id", |r| {
        let kind = text(r, "kind")?;
        // a row is kept under its listing's key; the symbol is the key's, the venue the
        // words it was read under
        let listing = text(r, "listing")?;
        let symbol = listing.rsplit_once('@').map_or(listing.as_str(), |(s, _)| s).to_string();
        Ok(NewsRow {
            id: text(r, "id")?,
            symbol,
            exchange: text(r, "exchange")?,
            headline: text(r, "headline")?,
            wire: text(r, "wire")?,
            url: text(r, "url")?,
            published_at: text(r, "published_at")?,
            kind: if kind.is_empty() { "story".to_string() } else { kind },
        })
    })
}

/// Each universe's constituents, the universes in key order.
pub fn universes(conn: &Connection) -> Result<Ordered<Vec<UniverseRow>>> {
    let rows = all(conn, "SELECT * FROM universes ORDER BY key, value DESC, symbol", |r| {
        Ok((
            text(r, "key")?,
            UniverseRow {
                symbol: text(r, "symbol")?,
                name: text(r, "name")?,
                value: number(r, "value")?,
                percent_change: maybe(r, "percent_change")?,
                sector: text(r, "sector")?,
                country: text(r, "country")?,
            },
        ))
    })?;
    let mut out: Vec<(String, Vec<UniverseRow>)> = Vec::new();
    for (key, row) in rows {
        match out.last_mut() {
            Some((k, list)) if *k == key => list.push(row),
            _ => out.push((key, vec![row])),
        }
    }
    Ok(Ordered(out))
}
