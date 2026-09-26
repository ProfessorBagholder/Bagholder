//! What the derived model was built from, as keys that are equal exactly when
//! nothing the model reads has changed.
//!
//! The keys are spelled from the store's generation counters
//! (`bagholder_store::gens`): one statement, and exact -- a counter moves only
//! when rows really differ, never because something was read again and found the
//! same. Two keys for the page: everything; and everything but the quotes
//! (prices move every minute and nothing else does, so a tick leaves the match,
//! the closed trades, the cashflow and the equity curve standing). The model's
//! own cache reads the counters one by one (`model_cache`).

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

