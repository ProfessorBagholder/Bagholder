//! The market data the store keeps: quotes, declared distributions, daily
//! price history and intraday bars, with the fetch stamps beside them.
//!
//! A closed day is written once and never rewritten. The newest stored day is
//! the exception, because a source can hand back a bar for a session still in
//! progress.

use rusqlite::{Connection, Result};
use serde_json::{json, Map, Value};

use bagholder_model::value::{field_s, get, num};

/// A number that is absent rather than zero.
fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(x) => {
            let n = num(Some(x), f64::NAN);
            if n.is_nan() { None } else { Some(n) }
        }
    }
}

fn sym_of(symbol: &str) -> String {
    symbol.trim().to_uppercase()
}

fn head10(s: &str) -> String {
    s.chars().take(10).collect()
}

// --------------------------------------------------------------------------
// distributions
// --------------------------------------------------------------------------

/// `distributions`: symbol -> the public record, newest first.
pub fn distributions(conn: &Connection) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare("SELECT * FROM distributions ORDER BY symbol, ex_date DESC")?;
    let mut rows = stmt.query([])?;
    let mut out: Map<String, Value> = Map::new();
    while let Some(r) = rows.next()? {
        let rec = json!({
            "exDate": r.get::<_, Option<String>>("ex_date")?.unwrap_or_default(),
            "payDate": r.get::<_, Option<String>>("pay_date")?.unwrap_or_default(),
            "amount": r.get::<_, Option<f64>>("amount")?,
            "currency": r.get::<_, Option<String>>("currency")?.unwrap_or_default(),
        });
        out.entry(r.get::<_, String>("symbol")?)
            .or_insert_with(|| Value::Array(vec![]))
            .as_array_mut()
            .unwrap()
            .push(rec);
    }
    Ok(out)
}

/// `upsert_distributions`: a record with no ex-date or no positive
/// amount is not a distribution.
pub fn upsert_distributions(conn: &Connection, symbol: &str, rows: &[Value], source: &str) -> Result<usize> {
    let sym = sym_of(symbol);
    if sym.is_empty() {
        return Ok(0);
    }
    let mut clean: Vec<(String, Option<String>, f64, Option<String>)> = Vec::new();
    for r in rows {
        let ex = head10(&field_s(r, "exDate"));
        let amt = opt_num(get(r, "amount"));
        match amt {
            Some(a) if ex.len() == 10 && a > 0.0 => {
                let pay = head10(&field_s(r, "payDate"));
                let ccy = field_s(r, "currency");
                clean.push((
                    ex,
                    if pay.is_empty() { None } else { Some(pay) },
                    a,
                    if ccy.is_empty() { None } else { Some(ccy) },
                ));
            }
            _ => continue,
        }
    }
    if clean.is_empty() {
        return Ok(0);
    }
    for (ex, pay, amt, ccy) in &clean {
        conn.execute(
            "INSERT INTO distributions(symbol, ex_date, pay_date, amount, currency, source) VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(symbol, ex_date, source) DO UPDATE SET pay_date = excluded.pay_date, amount = excluded.amount, currency = excluded.currency",
            rusqlite::params![sym, ex, pay, amt, ccy, source],
        )?;
    }
    Ok(clean.len())
}

// --------------------------------------------------------------------------
// quotes
// --------------------------------------------------------------------------

/// `quotes`.
pub fn quotes(conn: &Connection) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare("SELECT * FROM quotes")?;
    let mut rows = stmt.query([])?;
    let mut out = Map::new();
    while let Some(r) = rows.next()? {
        out.insert(
            r.get::<_, String>("symbol")?,
            json!({
                "price": r.get::<_, Option<f64>>("price")?,
                "priceChange": r.get::<_, Option<f64>>("price_change")?,
                "percentChange": r.get::<_, Option<f64>>("percent_change")?,
                "prevClose": r.get::<_, Option<f64>>("prev_close")?,
                "dividendAmount": r.get::<_, Option<f64>>("dividend_amount")?,
                "dividendFrequency": r.get::<_, Option<String>>("dividend_frequency")?.unwrap_or_default(),
                "exDividendDate": r.get::<_, Option<String>>("ex_dividend_date")?.unwrap_or_default(),
                "source": r.get::<_, Option<String>>("source")?.unwrap_or_default(),
                "fetchedAt": r.get::<_, Option<String>>("fetched_at")?.unwrap_or_default(),
            }),
        );
    }
    Ok(out)
}

/// `upsert_quote`: the price fields are replaced outright, but a
/// dividend figure already known is kept when the new quote does not carry
/// one -- a price feed that says nothing about dividends must not erase them.
pub fn upsert_quote(conn: &Connection, symbol: &str, rec: &Value, source: &str, now: &str) -> Result<()> {
    let sym = sym_of(symbol);
    if sym.is_empty() || !rec.is_object() {
        return Ok(());
    }
    let fetched = { let f = field_s(rec, "fetchedAt"); if f.is_empty() { now.to_string() } else { f } };
    conn.execute(
        "INSERT INTO quotes(symbol, price, price_change, percent_change, prev_close, dividend_amount, dividend_frequency, ex_dividend_date, source, fetched_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(symbol) DO UPDATE SET price = excluded.price, price_change = excluded.price_change, \
         percent_change = excluded.percent_change, prev_close = excluded.prev_close, \
         dividend_amount = COALESCE(excluded.dividend_amount, quotes.dividend_amount), \
         dividend_frequency = CASE WHEN excluded.dividend_frequency = '' THEN quotes.dividend_frequency ELSE excluded.dividend_frequency END, \
         ex_dividend_date = CASE WHEN excluded.ex_dividend_date = '' THEN quotes.ex_dividend_date ELSE excluded.ex_dividend_date END, \
         source = excluded.source, fetched_at = excluded.fetched_at",
        rusqlite::params![
            sym,
            opt_num(get(rec, "price")),
            opt_num(get(rec, "priceChange")),
            opt_num(get(rec, "percentChange")),
            opt_num(get(rec, "prevClose")),
            opt_num(get(rec, "dividendAmount")),
            field_s(rec, "dividendFrequency"),
            head10(&field_s(rec, "exDividendDate")),
            source,
            fetched,
        ],
    )?;
    Ok(())
}

fn stamp_map(conn: &Connection, sql: &str) -> Result<Map<String, Value>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Map::new();
    while let Some(r) = rows.next()? {
        out.insert(r.get::<_, String>(0)?, json!(r.get::<_, Option<String>>(1)?.unwrap_or_default()));
    }
    Ok(out)
}

pub fn quote_fetched_at(conn: &Connection) -> Result<Map<String, Value>> {
    stamp_map(conn, "SELECT symbol, fetched_at FROM quotes")
}

pub fn distributions_fetched_at(conn: &Connection) -> Result<Map<String, Value>> {
    stamp_map(conn, "SELECT symbol, fetched_at FROM distribution_fetches")
}

pub fn mark_distributions_fetched(conn: &Connection, symbol: &str, when: &str) -> Result<()> {
    let sym = sym_of(symbol);
    if sym.is_empty() || when.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO distribution_fetches(symbol, fetched_at) VALUES (?, ?) ON CONFLICT(symbol) DO UPDATE SET fetched_at = excluded.fetched_at",
        rusqlite::params![sym, when],
    )?;
    Ok(())
}

// --------------------------------------------------------------------------
// daily history
// --------------------------------------------------------------------------

/// `price_history`: daily bars for one symbol, oldest first.
pub fn price_history(conn: &Connection, symbol: &str, start: &str, end: &str) -> Result<Vec<Value>> {
    let sym = sym_of(symbol);
    if sym.is_empty() {
        return Ok(vec![]);
    }
    let from = { let s = head10(start); if s.is_empty() { "0000-01-01".to_string() } else { s } };
    let to = { let s = head10(end); if s.is_empty() { "9999-12-31".to_string() } else { s } };
    let mut stmt = conn.prepare(
        "SELECT date, open, high, low, close, volume FROM price_history WHERE symbol = ? AND date >= ? AND date <= ? ORDER BY date",
    )?;
    let mut rows = stmt.query(rusqlite::params![sym, from, to])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(json!({
            "date": r.get::<_, Option<String>>(0)?,
            "open": r.get::<_, Option<f64>>(1)?,
            "high": r.get::<_, Option<f64>>(2)?,
            "low": r.get::<_, Option<f64>>(3)?,
            "close": r.get::<_, Option<f64>>(4)?,
            "volume": r.get::<_, Option<f64>>(5)?,
        }));
    }
    Ok(out)
}

struct DayBar {
    date: String,
    open: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    close: f64,
    volume: Option<f64>,
}

/// `upsert_price_history`.
pub fn upsert_price_history(conn: &Connection, symbol: &str, bars: &[Value], source: &str) -> Result<usize> {
    let sym = sym_of(symbol);
    let mut clean: Vec<DayBar> = Vec::new();
    for b in bars {
        let d = head10(&field_s(b, "date"));
        let close = opt_num(get(b, "close"));
        let ok = d.len() == 10 && d.as_bytes()[4] == b'-' && close.map_or(false, |c| c > 0.0);
        if !ok {
            continue;
        }
        clean.push(DayBar {
            date: d,
            open: opt_num(get(b, "open")),
            high: opt_num(get(b, "high")),
            low: opt_num(get(b, "low")),
            close: close.unwrap(),
            volume: opt_num(get(b, "volume")),
        });
    }
    if sym.is_empty() || clean.is_empty() {
        return Ok(0);
    }
    let newest: String = conn
        .query_row("SELECT MAX(date) FROM price_history WHERE symbol = ?", [&sym], |r| {
            r.get::<_, Option<String>>(0)
        })?
        .unwrap_or_default();

    for c in &clean {
        conn.execute(
            "INSERT OR IGNORE INTO price_history(symbol, date, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![sym, c.date, c.open, c.high, c.low, c.close, c.volume, source],
        )?;
    }
    if !newest.is_empty() {
        // the session that was still open when it was first stored
        for c in clean.iter().filter(|c| c.date == newest) {
            conn.execute(
                "UPDATE price_history SET open = ?, high = ?, low = ?, close = ?, volume = ?, source = ? WHERE symbol = ? AND date = ?",
                rusqlite::params![c.open, c.high, c.low, c.close, c.volume, source, sym, c.date],
            )?;
        }
    }
    Ok(clean.len())
}

pub fn history_fetch(conn: &Connection, symbol: &str) -> Result<Value> {
    let sym = sym_of(symbol);
    let mut stmt = conn.prepare("SELECT start, fetched_at FROM history_fetches WHERE symbol = ?")?;
    let mut rows = stmt.query([&sym])?;
    match rows.next()? {
        Some(r) => Ok(json!({
            "start": r.get::<_, Option<String>>(0)?,
            "fetchedAt": r.get::<_, Option<String>>(1)?,
        })),
        None => Ok(Value::Null),
    }
}

/// `mark_history_fetched`: the stamp only ever reaches further back.
pub fn mark_history_fetched(conn: &Connection, symbol: &str, start: &str, when: &str) -> Result<()> {
    let sym = sym_of(symbol);
    if sym.is_empty() || when.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO history_fetches(symbol, start, fetched_at) VALUES (?, ?, ?) ON CONFLICT(symbol) DO UPDATE SET start = MIN(history_fetches.start, excluded.start), fetched_at = excluded.fetched_at",
        rusqlite::params![sym, head10(start), when],
    )?;
    Ok(())
}

// --------------------------------------------------------------------------
// intraday bars
// --------------------------------------------------------------------------

/// `price_bars`.
pub fn price_bars(conn: &Connection, symbol: &str, tf: &str, start_ts: i64, end_ts: i64) -> Result<Vec<Value>> {
    let sym = sym_of(symbol);
    let mut stmt = conn.prepare(
        "SELECT ts, open, high, low, close, volume FROM price_bars WHERE symbol = ? AND tf = ? AND ts >= ? AND ts <= ? ORDER BY ts",
    )?;
    let mut rows = stmt.query(rusqlite::params![sym, tf, start_ts, end_ts])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(json!({
            "time": r.get::<_, i64>(0)?,
            "open": r.get::<_, Option<f64>>(1)?,
            "high": r.get::<_, Option<f64>>(2)?,
            "low": r.get::<_, Option<f64>>(3)?,
            "close": r.get::<_, Option<f64>>(4)?,
            "volume": r.get::<_, Option<f64>>(5)?,
        }));
    }
    Ok(out)
}

/// `upsert_price_bars`.
pub fn upsert_price_bars(conn: &Connection, symbol: &str, tf: &str, bars: &[Value], source: &str) -> Result<usize> {
    let sym = sym_of(symbol);
    struct Bar { ts: i64, open: Option<f64>, high: Option<f64>, low: Option<f64>, close: f64, volume: Option<f64> }
    let mut clean: Vec<Bar> = Vec::new();
    for b in bars {
        let time = get(b, "time");
        let close = opt_num(get(b, "close"));
        // a zero close is rejected too
        match (time, close) {
            (Some(t), Some(c)) if c > 0.0 => clean.push(Bar {
                ts: num(Some(t), 0.0) as i64,
                open: opt_num(get(b, "open")),
                high: opt_num(get(b, "high")),
                low: opt_num(get(b, "low")),
                close: c,
                volume: opt_num(get(b, "volume")),
            }),
            _ => continue,
        }
    }
    if sym.is_empty() || clean.is_empty() {
        return Ok(0);
    }
    let newest: Option<i64> = conn.query_row(
        "SELECT MAX(ts) FROM price_bars WHERE symbol = ? AND tf = ?",
        rusqlite::params![sym, tf],
        |r| r.get(0),
    )?;
    for c in &clean {
        conn.execute(
            "INSERT OR IGNORE INTO price_bars(symbol, tf, ts, open, high, low, close, volume, source) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![sym, tf, c.ts, c.open, c.high, c.low, c.close, c.volume, source],
        )?;
    }
    if let Some(n) = newest {
        for c in clean.iter().filter(|c| c.ts == n) {
            conn.execute(
                "UPDATE price_bars SET open = ?, high = ?, low = ?, close = ?, volume = ?, source = ? WHERE symbol = ? AND tf = ? AND ts = ?",
                rusqlite::params![c.open, c.high, c.low, c.close, c.volume, source, sym, tf, c.ts],
            )?;
        }
    }
    Ok(clean.len())
}

/// `last_bar_time`: one indexed lookup, rather than reading the archive
/// to look at its last row.
pub fn last_bar_time(conn: &Connection, symbol: &str, tf: &str) -> Result<Option<i64>> {
    let sym = sym_of(symbol);
    conn.query_row(
        "SELECT MAX(ts) FROM price_bars WHERE symbol = ? AND tf = ?",
        rusqlite::params![sym, tf],
        |r| r.get(0),
    )
}

pub fn bar_fetch(conn: &Connection, symbol: &str, tf: &str) -> Result<Value> {
    let sym = sym_of(symbol);
    let mut stmt = conn.prepare("SELECT start_ts, fetched_at FROM bar_fetches WHERE symbol = ? AND tf = ?")?;
    let mut rows = stmt.query(rusqlite::params![sym, tf])?;
    match rows.next()? {
        Some(r) => Ok(json!({
            "startTs": r.get::<_, Option<i64>>(0)?,
            "fetchedAt": r.get::<_, Option<String>>(1)?,
        })),
        None => Ok(Value::Null),
    }
}

pub fn mark_bars_fetched(conn: &Connection, symbol: &str, tf: &str, start_ts: i64, when: &str) -> Result<()> {
    let sym = sym_of(symbol);
    if sym.is_empty() || when.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO bar_fetches(symbol, tf, start_ts, fetched_at) VALUES (?, ?, ?, ?) ON CONFLICT(symbol, tf) DO UPDATE SET start_ts = MIN(bar_fetches.start_ts, excluded.start_ts), fetched_at = excluded.fetched_at",
        rusqlite::params![sym, tf, start_ts, when],
    )?;
    Ok(())
}

/// `BENCHMARK_SYMBOLS`.
pub const BENCHMARK_SYMBOLS: [&str; 3] = ["SP500", "TSX", "TSX60"];

/// `market_data`: what `build_base` is handed.
pub fn market_data(conn: &Connection) -> Result<Value> {
    let mut benchmarks = Map::new();
    for sym in BENCHMARK_SYMBOLS.iter() {
        benchmarks.insert((*sym).to_string(), Value::Object(crate::tables::benchmark_prices(conn, sym)?));
    }
    Ok(json!({
        "fx": crate::tables::fx_rates(conn, crate::tables::FX_PAIR)?,
        "benchmark": crate::tables::benchmark_prices(conn, crate::tables::BENCHMARK_SYMBOL)?,
        "benchmarks": benchmarks,
        "distributions": distributions(conn)?,
        "quotes": quotes(conn)?,
    }))
}
