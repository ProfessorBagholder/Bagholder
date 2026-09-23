//! The two parts of the model's inputs still read as JSON here: exposures
//! (an id-keyed map of weight maps, genuinely free-form) and the market
//! universes (heatmaps beyond the book). Both have callers beyond the store
//! that still want that shape.

use rusqlite::{Connection, Result, Row};
use serde_json::{json, Map, Value};

fn text(row: &Row, name: &str) -> rusqlite::Result<String> {
    Ok(row.get::<_, Option<String>>(name)?.unwrap_or_default())
}

fn real(row: &Row, name: &str) -> rusqlite::Result<Value> {
    Ok(match row.get::<_, Option<f64>>(name)? {
        Some(v) => json!(v),
        None => Value::Null,
    })
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

pub fn exposures_part(conn: &Connection) -> Result<Map<String, Value>> {
    let mut exposures: Map<String, Value> = Map::new();
    let mut stmt = conn.prepare("SELECT * FROM exposures")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        exposures.insert(text(r, "key")?, serde_json::to_value(crate::feeds::stored_exposure(r)?).unwrap_or(Value::Null));
    }
    Ok(exposures)
}

pub fn universes_part(conn: &Connection) -> Result<Map<String, Value>> {
    universes(conn)
}
