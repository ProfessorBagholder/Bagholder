//! A cheap fingerprint of everything the derived model reads.
//!
//! The first value says whether the model is current at all. The second leaves
//! the quotes out: when nothing but a price has moved, the match, the closed
//! trades, the cashflow and the equity curve are all still good and only the
//! open positions have to be marked again.

use rusqlite::{Connection, Result};

/// The tables and meta keys the derived model reads. `quotes` is kept apart
/// because prices move every minute and nothing else does.
const VERSION_SQL: [&str; 14] = [
    "SELECT COUNT(*), MAX(COALESCE(occurred_at, transaction_date)) FROM activities",
    "SELECT COUNT(*), MAX(date) FROM nav_history",
    "SELECT COUNT(*), MAX(date) FROM fx_rates",
    "SELECT COUNT(*), MAX(date) FROM benchmark_prices",
    "SELECT COUNT(*), MAX(ex_date) FROM distributions",
    "SELECT COUNT(*), MAX(fetched_at) FROM securities",
    "SELECT COUNT(*), SUM(quantity) FROM balances",
    "SELECT COUNT(*), MAX(id) FROM accounts",
    "SELECT COUNT(*), MAX(fetched_at) FROM margin",
    "SELECT COUNT(*), MAX(fetched_at) FROM exposures",
    "SELECT COUNT(*), MAX(added_at) FROM watchlist",
    "SELECT COUNT(*), MAX(fetched_at) FROM news",
    "SELECT COUNT(*), MAX(fetched_at) FROM universes",
    "SELECT COUNT(*), SUM(COALESCE(net_liquidation_value, 0)) FROM accounts",
];

/// The prices themselves, not only the stamp: two quotes written in the same
/// second used to leave the fingerprint unchanged and the page kept the old
/// price.
const QUOTES_SQL: &str = "SELECT COUNT(*), MAX(fetched_at), TOTAL(price) FROM quotes";

const VERSION_META: [&str; 5] = ["synced_at", "trade_groups", "trade_notes", "journal_v2", "market_tiles"];

/// One scalar as the fingerprint spells it. SQLite hands back whatever the
/// column held, printed as text (`None` for null, `1.0` for a whole real).
fn scalar(conn: &Connection, sql: &str, idx: usize) -> Result<String> {
    conn.query_row(sql, [], |r| {
        Ok(match r.get_ref(idx)? {
            rusqlite::types::ValueRef::Null => "None".to_string(),
            rusqlite::types::ValueRef::Integer(n) => n.to_string(),
            rusqlite::types::ValueRef::Real(f) => {
                if f.fract() == 0.0 && f.abs() < 1e16 { format!("{:.1}", f) } else { f.to_string() }
            }
            rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).to_string(),
            rusqlite::types::ValueRef::Blob(_) => String::new(),
        })
    })
}

/// FNV-1a over the stored text.
///
/// The fingerprint is only ever compared with another taken by the same
/// run, so any hash would do; this one is stable across runs.
fn text_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// `(everything, everything but the quotes)`, in one pass.
pub fn versions(conn: &Connection) -> Result<(String, String)> {
    let mut parts: Vec<String> = Vec::new();
    for sql in VERSION_SQL {
        parts.push(format!("{}:{}", scalar(conn, sql, 0)?, scalar(conn, sql, 1)?));
    }
    for key in VERSION_META {
        let val = bagholder_store::tables::get_meta(conn, key, "")?;
        parts.push(format!("{}:{}:{}", key, val.chars().count(), text_hash(&val)));
    }
    let core = parts.join("|");
    let q = format!(
        "q:{}:{}:{}",
        scalar(conn, QUOTES_SQL, 0)?,
        scalar(conn, QUOTES_SQL, 1)?,
        scalar(conn, QUOTES_SQL, 2)?
    );
    Ok((format!("{}|{}", core, q), core))
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
    let acts = format!(
        "{}:{}",
        scalar(conn, VERSION_SQL[0], 0)?,
        scalar(conn, VERSION_SQL[0], 1)?
    );
    let secs = format!("{}:{}", scalar(conn, VERSION_SQL[5], 0)?, scalar(conn, VERSION_SQL[5], 1)?);
    Ok(format!("{}|{}", acts, secs))
}

/// What the header needs, read without counting the
/// tables themselves.
pub fn status_counts(conn: &Connection) -> Result<(i64, i64, String)> {
    let acts: i64 = conn.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0))?;
    let accounts: i64 = conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
    let synced = bagholder_store::tables::get_meta(conn, "synced_at", "")?;
    Ok((acts, accounts, synced))
}
