//! What the derived model was built from, as keys that are equal exactly when
//! nothing the model reads from the market cache has changed.
//!
//! The keys are spelled from the market cache's generation counters
//! (`bagholder_market::stored::gens`): one statement, and exact -- a counter moves
//! only when rows really differ, never because something was read again and found
//! the same. Two keys for the page: everything; and everything but the quotes,
//! which no counter counts (the cache's quotes reach the page as figures), so a
//! price tick leaves both standing.

use rusqlite::{Connection, Result};

use bagholder_market::stored::gens;

/// `(everything, everything but the quotes)`, in one pass.
pub fn versions(conn: &Connection) -> Result<(String, String)> {
    let key = gens::key(&gens::all(conn)?, &gens::names());
    Ok((key.clone(), key))
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
