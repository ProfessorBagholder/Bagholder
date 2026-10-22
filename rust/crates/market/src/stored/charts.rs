//! The charts' series the market cache keeps: each listing's daily price history
//! and its intraday bars, with the fetch stamps beside them.
//!
//! A closed day is written once and never rewritten. The newest stored day is
//! the exception, because a source can hand back a bar for a session still in
//! progress.

use bagholder_core::ListingKey;
use rusqlite::{Connection, Result};

use super::bars::{BarFetch, DayBar, HistoryFetch, Ohlcv, TimeBar};

fn head10(s: &str) -> String {
    s.chars().take(10).collect()
}

// --------------------------------------------------------------------------
// daily history
// --------------------------------------------------------------------------

/// `price_history`: daily bars for one symbol, oldest first. A row whose
/// close was never written (an old source's leftover) is skipped.
pub fn price_history(conn: &Connection, listing: &ListingKey, start: &str, end: &str) -> Result<Vec<DayBar>> {
    let sym = listing.to_string();
    let from = { let s = head10(start); if s.is_empty() { "0000-01-01".to_string() } else { s } };
    let to = { let s = head10(end); if s.is_empty() { "9999-12-31".to_string() } else { s } };
    let mut stmt = conn.prepare(
        "SELECT date, open, high, low, close, volume FROM price_history WHERE listing = ? AND date >= ? AND date <= ? ORDER BY date",
    )?;
    let mut rows = stmt.query(rusqlite::params![sym, from, to])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let close: Option<f64> = r.get(4)?;
        let close = match close { Some(c) => c, None => continue };
        out.push(DayBar {
            date: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
            px: Ohlcv { open: r.get(1)?, high: r.get(2)?, low: r.get(3)?, close, volume: r.get(5)? },
        });
    }
    Ok(out)
}

/// `upsert_price_history`.
pub fn upsert_price_history(conn: &Connection, listing: &ListingKey, bars: &[DayBar], source: &str) -> Result<usize> {
    bagholder_sqlite::atomically(conn, || {
        let sym = listing.to_string();
        let mut clean: Vec<DayBar> = Vec::new();
        for b in bars {
            let d = head10(&b.date);
            let ok = d.len() == 10 && d.as_bytes()[4] == b'-' && b.px.close > 0.0;
            if !ok {
                continue;
            }
            clean.push(DayBar { date: d, px: b.px });
        }
        if clean.is_empty() {
            return Ok(0);
        }
        let newest: String = conn
            .query_row("SELECT MAX(date) FROM price_history WHERE listing = ?", [&sym], |r| {
                r.get::<_, Option<String>>(0)
            })?
            .unwrap_or_default();

        for c in &clean {
            conn.execute(
                "INSERT OR IGNORE INTO price_history(listing, date, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![sym, c.date, c.px.open, c.px.high, c.px.low, c.px.close, c.px.volume, source],
            )?;
        }
        if !newest.is_empty() {
            // the session that was still open when it was first stored
            for c in clean.iter().filter(|c| c.date == newest) {
                conn.execute(
                    "UPDATE price_history SET open = ?, high = ?, low = ?, close = ?, volume = ?, source = ? WHERE listing = ? AND date = ?",
                    rusqlite::params![c.px.open, c.px.high, c.px.low, c.px.close, c.px.volume, source, sym, c.date],
                )?;
            }
        }
        Ok(clean.len())
    })
}

pub fn history_fetch(conn: &Connection, listing: &ListingKey) -> Result<Option<HistoryFetch>> {
        let sym = listing.to_string();
    let mut stmt = conn.prepare("SELECT start, fetched_at FROM history_fetches WHERE listing = ?")?;
    let mut rows = stmt.query([&sym])?;
    match rows.next()? {
        Some(r) => Ok(Some(HistoryFetch {
            start: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
            fetched_at: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
        })),
        None => Ok(None),
    }
}

/// `mark_history_fetched`: the stamp only ever reaches further back.
pub fn mark_history_fetched(conn: &Connection, listing: &ListingKey, start: &str, when: &str) -> Result<()> {
    let sym = listing.to_string();
    if when.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO history_fetches(listing, start, fetched_at) VALUES (?, ?, ?) ON CONFLICT(listing) DO UPDATE SET start = MIN(history_fetches.start, excluded.start), fetched_at = excluded.fetched_at",
        rusqlite::params![sym, head10(start), when],
    )?;
    Ok(())
}

// --------------------------------------------------------------------------
// intraday bars
// --------------------------------------------------------------------------

/// `price_bars`.
pub fn price_bars(conn: &Connection, listing: &ListingKey, tf: &str, start_ts: i64, end_ts: i64) -> Result<Vec<TimeBar>> {
        let sym = listing.to_string();
    let mut stmt = conn.prepare(
        "SELECT ts, open, high, low, close, volume FROM price_bars WHERE listing = ? AND tf = ? AND ts >= ? AND ts <= ? ORDER BY ts",
    )?;
    let mut rows = stmt.query(rusqlite::params![sym, tf, start_ts, end_ts])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let close: Option<f64> = r.get(4)?;
        let close = match close { Some(c) => c, None => continue };
        out.push(TimeBar {
            time: r.get(0)?,
            px: Ohlcv { open: r.get(1)?, high: r.get(2)?, low: r.get(3)?, close, volume: r.get(5)? },
        });
    }
    Ok(out)
}

/// `upsert_price_bars`.
pub fn upsert_price_bars(conn: &Connection, listing: &ListingKey, tf: &str, bars: &[TimeBar], source: &str) -> Result<usize> {
    bagholder_sqlite::atomically(conn, || {
        let sym = listing.to_string();
        // a zero close is rejected too
        let clean: Vec<&TimeBar> = bars.iter().filter(|b| b.px.close > 0.0).collect();
        if clean.is_empty() {
            return Ok(0);
        }
        let newest: Option<i64> = conn.query_row(
            "SELECT MAX(ts) FROM price_bars WHERE listing = ? AND tf = ?",
            rusqlite::params![sym, tf],
            |r| r.get(0),
        )?;
        for c in &clean {
            conn.execute(
                "INSERT OR IGNORE INTO price_bars(listing, tf, ts, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![sym, tf, c.time, c.px.open, c.px.high, c.px.low, c.px.close, c.px.volume, source],
            )?;
        }
        if let Some(n) = newest {
            for c in clean.iter().filter(|c| c.time == n) {
                conn.execute(
                    "UPDATE price_bars SET open = ?, high = ?, low = ?, close = ?, volume = ?, source = ? WHERE listing = ? AND tf = ? AND ts = ?",
                    rusqlite::params![c.px.open, c.px.high, c.px.low, c.px.close, c.px.volume, source, sym, tf, c.time],
                )?;
            }
        }
        Ok(clean.len())
    })
}

/// `last_bar_time`: one indexed lookup, rather than reading the archive
/// to look at its last row.
pub fn last_bar_time(conn: &Connection, listing: &ListingKey, tf: &str) -> Result<Option<i64>> {
        let sym = listing.to_string();
    conn.query_row(
        "SELECT MAX(ts) FROM price_bars WHERE listing = ? AND tf = ?",
        rusqlite::params![sym, tf],
        |r| r.get(0),
    )
}

pub fn bar_fetch(conn: &Connection, listing: &ListingKey, tf: &str) -> Result<Option<BarFetch>> {
        let sym = listing.to_string();
    let mut stmt = conn.prepare("SELECT start_ts, fetched_at FROM bar_fetches WHERE listing = ? AND tf = ?")?;
    let mut rows = stmt.query(rusqlite::params![sym, tf])?;
    match rows.next()? {
        Some(r) => Ok(Some(BarFetch {
            start_ts: r.get(0)?,
            fetched_at: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
        })),
        None => Ok(None),
    }
}

pub fn mark_bars_fetched(conn: &Connection, listing: &ListingKey, tf: &str, start_ts: i64, when: &str) -> Result<()> {
    let sym = listing.to_string();
    if when.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO bar_fetches(listing, tf, start_ts, fetched_at) VALUES (?, ?, ?, ?) ON CONFLICT(listing, tf) DO UPDATE SET start_ts = MIN(bar_fetches.start_ts, excluded.start_ts), fetched_at = excluded.fetched_at",
        rusqlite::params![sym, tf, start_ts, when],
    )?;
    Ok(())
}
