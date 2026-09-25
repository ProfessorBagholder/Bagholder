//! Relabelling the option rows Wealthsimple stores under its own names.
//!
//! Sync never replaces a stored row, so rows land with the broker's labels and
//! have to be relabelled after every pull. The raw rows are not rewritten
//! beyond this: these are the labels the whole model reads a row's meaning
//! from, and they are corrected in place once, not re-derived on every read.

use rusqlite::{Connection, Result};

pub const OPTION_RELABEL_META: &str = "option_relabel_rows_v1";
pub const OPTION_UNIT_PRICE_SCALE_META: &str = "option_unit_price_scale_v1";

const RAW: &str = "UPPER(REPLACE(IFNULL(raw_type,''), '-', '_'))";

/// `_relabel_when_rows_changed`: relabel newly arrived option rows and
/// nothing else.
///
/// Doing this on every read meant six UPDATEs over the whole table each time
/// the page asked for anything, so the fingerprint of the rows is stamped once
/// they are relabelled and an unchanged table is left alone.
pub fn relabel_when_rows_changed(conn: &Connection) -> Result<bool> {
    let (n, m): (i64, Option<String>) = conn.query_row(
        "SELECT COUNT(*) AS n, MAX(COALESCE(occurred_at, transaction_date)) AS m FROM activities",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // a missing maximum is written as the string "None"
    let key = format!("{}|{}", n, m.unwrap_or_else(|| "None".into()));
    let stamped: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?", [OPTION_RELABEL_META], |r| r.get(0))
        .ok();
    if stamped.as_deref() == Some(key.as_str()) {
        return Ok(false);
    }
    relabel_option_trades(conn)?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![OPTION_RELABEL_META, key],
    )?;
    Ok(true)
}

/// `_relabel_option_trades`: OPTIONS_BUY / OPTIONS_SELL were stored as
/// LIMIT_ORDER and the like. They are trades.
pub fn relabel_option_trades(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "UPDATE activities SET activity_sub_type = 'BUYTOOPEN', category = 'trade', quantity = ABS(quantity), net_cash_amount = -ABS(net_cash_amount) \
         WHERE {RAW} = 'OPTIONS_BUY' \
         AND UPPER(REPLACE(IFNULL(activity_sub_type,''), '-', '_')) NOT IN ('BUY', 'BUYTOOPEN', 'BTO', 'BUYTOCLOSE', 'BTC');
         UPDATE activities SET activity_sub_type = 'SELLTOOPEN', category = 'trade', quantity = -ABS(quantity), net_cash_amount = ABS(net_cash_amount) \
         WHERE {RAW} = 'OPTIONS_SELL' \
         AND UPPER(REPLACE(IFNULL(activity_sub_type,''), '-', '_')) NOT IN ('SELL', 'SELLTOOPEN', 'STO', 'SELLTOCLOSE', 'STC', 'COVER');"
    ))?;
    relabel_option_closes(conn)
}

/// `_relabel_option_closes`: the close and open semantics of
/// OPTIONS_MULTILEG, expiry and assignment rows.
pub fn relabel_option_closes(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "UPDATE activities SET activity_type = 'OPTIONS_BUY', activity_sub_type = 'BUYTOCLOSE', category = 'trade' \
         WHERE {RAW} LIKE '%MULTILEG%' AND IFNULL(net_cash_amount, 0) < 0;
         UPDATE activities SET activity_type = 'OPTIONS_SELL', activity_sub_type = 'SELLTOOPEN', category = 'trade' \
         WHERE {RAW} LIKE '%MULTILEG%' AND IFNULL(net_cash_amount, 0) >= 0;
         UPDATE activities SET activity_type = 'ASSIGN', activity_sub_type = 'BUYTOCLOSE', category = 'option_event', quantity = ABS(quantity), unit_price = 0 \
         WHERE {RAW} LIKE '%ASSIGN%';
         UPDATE activities SET activity_type = 'EXPIR', activity_sub_type = 'BUY', category = 'option_event', quantity = ABS(quantity), \
         unit_price = CASE WHEN ABS(IFNULL(net_cash_amount, 0)) < 1e-12 THEN 0 ELSE unit_price END \
         WHERE {RAW} LIKE '%SHORT%EXPIR%';
         UPDATE activities SET activity_type = 'EXPIR', activity_sub_type = 'SELL', category = 'option_event', quantity = -ABS(quantity), \
         unit_price = CASE WHEN ABS(IFNULL(net_cash_amount, 0)) < 1e-12 THEN 0 ELSE unit_price END \
         WHERE {RAW} LIKE '%EXPIR%' AND {RAW} NOT LIKE '%SHORT%';"
    ))
}

/// `_is_option_symbol`: the store's own cheaper reading, which is not
/// the model's -- it only has to spot the shapes the price scaling cares about.
fn is_option_symbol(symbol: &str) -> bool {
    let compact = symbol.trim().to_uppercase();
    if compact.is_empty() {
        return false;
    }
    let padded = format!(" {} ", compact);
    if padded.contains(" CALL ") || padded.contains(" PUT ") {
        return true;
    }
    compact.ends_with(" C") || compact.ends_with(" P")
}

/// `_cash_near`.
fn cash_near(a: f64, b: f64) -> bool {
    (a - b).abs() <= f64::max(0.02, 0.02 * f64::max(f64::max(a.abs(), b.abs()), 1e-9))
}

/// `_scale_option_unit_prices`: one-shot, divide an option's unit price
/// by a hundred when the cash says the stored price was contract cash.
///
/// Wealthsimple's option amount is contract cash, so the per-share price is
/// amount / (qty x 100). An earlier `unit_price > 20` rule left cheap
/// contracts a hundred times too high. Rows that are already right are left
/// alone.
pub fn scale_option_unit_prices(conn: &Connection) -> Result<()> {
    crate::atomically(conn, || {
        let mut stmt = conn.prepare("SELECT id, symbol, quantity, unit_price, net_cash_amount FROM activities")?;
        let rows: Vec<(String, String, f64, f64, f64)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                    r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                    r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                ))
            })?
            .collect::<Result<Vec<_>>>()?;
        drop(stmt);

        for (id, symbol, quantity, unit_price, cash) in rows {
            if !is_option_symbol(&symbol) {
                continue;
            }
            let (qty, px, cash) = (quantity.abs(), unit_price.abs(), cash.abs());
            if qty <= 0.0 || px <= 0.0 || cash <= 0.0 {
                continue;
            }
            let implied = px * qty;
            if cash_near(cash, implied * 100.0) {
                continue;
            }
            if cash_near(cash, implied) {
                conn.execute("UPDATE activities SET unit_price = ? WHERE id = ?", rusqlite::params![px / 100.0, id])?;
            }
        }
        Ok(())
    })
}

/// `ensure`: the schema, then the relabelling, then the one-shot price
/// scaling stamped so it never runs twice.
pub fn ensure(conn: &Connection) -> Result<()> {
    crate::schema::init_schema(conn)?;
    // the rows these repairs mend are the book's now
    if crate::schema::figures_moved(conn)? {
        return Ok(());
    }
    relabel_when_rows_changed(conn)?;
    let stamped: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?", [OPTION_UNIT_PRICE_SCALE_META], |r| r.get(0))
        .ok();
    if stamped.is_none() {
        scale_option_unit_prices(conn)?;
        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![OPTION_UNIT_PRICE_SCALE_META, "1"],
        )?;
    }
    Ok(())
}
