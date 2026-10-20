//! Generations: for each table of the market cache a figure or a feed reads, a
//! counter that moves when, and only when, its rows actually change.
//!
//! The counters are kept by the database itself, in triggers the cache's
//! migrations make (`bagholder_sources::cache::MIGRATIONS`), so no writer can
//! forget one and none can move one for nothing: an insert and a delete move the
//! table's counter; an update moves it only when a column that is data differs
//! (`IS NOT`, so a null is compared too), never for a bookkeeping stamp alone.
//! (docs/architecture.md, rules 3 and 4.)

use rusqlite::{Connection, Result};
use std::collections::BTreeMap;

/// Every counter the market cache keeps, as its migrations make them.
pub fn names() -> Vec<&'static str> {
    vec!["exposures", "filings", "news", "universes"]
}

/// Every counter, by name, in one statement.
pub fn all(conn: &Connection) -> Result<BTreeMap<String, i64>> {
    let mut stmt = conn.prepare_cached("SELECT name, n FROM gen")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<Result<BTreeMap<_, _>>>()?;
    Ok(rows)
}

/// The counters named, spelled as one key: equal keys mean nothing among them
/// changed.
pub fn key(gens: &BTreeMap<String, i64>, of: &[&str]) -> String {
    of.iter().map(|n| format!("{}:{}", n, gens.get(*n).copied().unwrap_or(0))).collect::<Vec<_>>().join("|")
}

fn rows_of(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Vec<Vec<String>>> {
    let mut stmt = conn.prepare(sql)?;
    let n = stmt.column_count();
    let mut out = Vec::new();
    let mut q = stmt.query(params)?;
    while let Some(r) = q.next()? {
        out.push((0..n).map(|i| r.get_ref(i).map(|v| format!("{:?}", v))).collect::<Result<Vec<_>>>()?);
    }
    out.sort();
    Ok(out)
}

/// Run a writer that replaces a set of rows (delete them, insert the new ones),
/// and keep what it wrote only if the set is different afterwards. `watch`
/// selects the data columns of the rows in question -- never a bookkeeping stamp
/// -- and the rows are compared as the database holds them, so "the same" is
/// exact and needs no second copy of the writer's own conversions. When nothing
/// differs the writes are undone, the counters with them, and `false` comes
/// back: the caller re-stamps whatever says when the rows were last read.
/// Must run inside `atomically`.
pub fn replace_if_changed(conn: &Connection, watch: &str, params: &[&dyn rusqlite::ToSql], write: impl FnOnce() -> Result<()>) -> Result<bool> {
    debug_assert!(!conn.is_autocommit(), "replace_if_changed runs inside atomically");
    let before = rows_of(conn, watch, params)?;
    conn.execute_batch("SAVEPOINT replace_if_changed")?;
    let done = write().and_then(|_| rows_of(conn, watch, params));
    match done {
        Ok(after) if after != before => {
            conn.execute_batch("RELEASE replace_if_changed")?;
            Ok(true)
        }
        Ok(_) => {
            conn.execute_batch("ROLLBACK TO replace_if_changed; RELEASE replace_if_changed")?;
            Ok(false)
        }
        Err(e) => {
            // a savepoint that cannot be undone leaves the connection mid-write:
            // that failure is the one said
            conn.execute_batch("ROLLBACK TO replace_if_changed; RELEASE replace_if_changed")?;
            Err(e)
        }
    }
}
