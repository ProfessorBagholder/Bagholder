//! What the derived model was built from, as keys that are equal exactly when
//! nothing the model reads has changed.
//!
//! The keys are spelled from the store's generation counters
//! (`bagholder_store::gens`): one statement, and exact -- a counter moves only
//! when rows really differ, never because something was read again and found the
//! same. Three keys, from the whole to the part: everything; everything but the
//! quotes (prices move every minute and nothing else does, so a tick leaves the
//! match, the closed trades, the cashflow and the equity curve standing); and
//! the rows the FIFO match itself reads.

use rusqlite::{Connection, Result};

use bagholder_store::gens;

/// `(everything, everything but the quotes)`, in one pass.
pub fn versions(conn: &Connection) -> Result<(String, String)> {
    let g = gens::all(conn)?;
    let names = gens::names();
    let core: Vec<&str> = names.iter().copied().filter(|n| *n != "quotes").collect();
    Ok((gens::key(&g, &names), gens::key(&g, &core)))
}

pub fn data_version(conn: &Connection) -> Result<String> {
    Ok(versions(conn)?.0)
}

/// Everything the model reads except the quotes. Unchanged across a pure price
/// tick, which is how the page tells a quote tick from a structural change and
/// fetches just the live figures instead of the whole book.
pub fn core_version(conn: &Connection) -> Result<String> {
    Ok(versions(conn)?.1)
}

/// The rows the FIFO match itself depends on.
pub fn book_version(conn: &Connection) -> Result<String> {
    Ok(gens::key(&gens::all(conn)?, &["activities", "securities"]))
}

/// What the header needs, read without counting the
/// tables themselves.
pub fn status_counts(conn: &Connection) -> Result<(i64, i64, String)> {
    let acts: i64 = conn.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0))?;
    let accounts: i64 = conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
    let synced = bagholder_store::tables::get_meta(conn, "synced_at", "")?;
    Ok((acts, accounts, synced))
}
