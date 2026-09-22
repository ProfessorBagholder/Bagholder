//! What the model is built from, read from the columns straight into the model's
//! own types, with no JSON between the row and the struct. Each reads what the
//! part of the same name in `snapshot` reads, and reads it as the model's lenient
//! reading of that JSON would: a text that is not there is empty, a number that
//! must be one is zero, and one that may be absent stays absent
//! (`tests/rows.rs` holds the two readings equal). What is kept as JSON text (the
//! journal, the groups, the tiles, an exposure's weights) is still parsed and
//! cleaned as it is for the snapshot, then read once into its type.

use rusqlite::{Connection, Result, Row};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

use bagholder_model::exposure::{Exposure, Exposures};
use bagholder_model::fx::Fx;
use bagholder_model::input::{AccountRow, BalanceRow, Distribution, Journal, MarginRow, NewsRow, Quote, Quotes, TileRef, TradeGroup, UniverseRow, WatchRow};
use bagholder_model::nav::NavRow;
use bagholder_model::securities::Security;
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

fn or_cad(c: String) -> String {
    if c.is_empty() { "CAD".to_string() } else { c }
}

fn all<T>(conn: &Connection, sql: &str, f: impl Fn(&Row) -> Result<T>) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| f(r))?;
    rows.collect()
}

pub fn securities(conn: &Connection) -> Result<Vec<Security>> {
    all(conn, "SELECT * FROM securities ORDER BY id", |r| {
        Ok(Security {
            id: text(r, "id")?,
            symbol: text(r, "symbol")?,
            name: text(r, "name")?,
            primary_exchange: text(r, "primary_exchange")?,
            primary_mic: text(r, "primary_mic")?,
            currency: text(r, "currency")?,
            underlying_id: text(r, "underlying_id")?,
        })
    })
}

pub fn accounts(conn: &Connection) -> Result<Vec<AccountRow>> {
    all(conn, "SELECT * FROM accounts ORDER BY id", |r| {
        Ok(AccountRow {
            id: text(r, "id")?,
            nickname: text(r, "nickname")?,
            unified_account_type: text(r, "unified_account_type")?,
            currency: text(r, "currency")?,
            status: text(r, "status")?,
            kind: text(r, "type")?,
            net_liquidation_value: maybe(r, "net_liquidation_value")?,
        })
    })
}

pub fn balances(conn: &Connection) -> Result<Vec<BalanceRow>> {
    // no ORDER BY: the rowid order is the order
    all(conn, "SELECT * FROM balances", |r| {
        Ok(BalanceRow { account_id: text(r, "account_id")?, security_id: text(r, "security_id")?, quantity: number(r, "quantity")? })
    })
}

pub fn margin(conn: &Connection) -> Result<Vec<MarginRow>> {
    all(conn, "SELECT * FROM margin ORDER BY account_id", |r| {
        Ok(MarginRow { account_id: text(r, "account_id")?, buying_power: maybe(r, "buying_power")?, currency: or_cad(text(r, "currency")?) })
    })
}

/// The NAV history: every account together, and each account's own.
pub fn nav(conn: &Connection) -> Result<(Vec<NavRow>, HashMap<String, Vec<NavRow>>)> {
    let mut together = Vec::new();
    let mut by_account: HashMap<String, Vec<NavRow>> = HashMap::new();
    let rows = all(conn, "SELECT * FROM nav_history ORDER BY account_id, date", |r| {
        Ok((text(r, "account_id")?, NavRow { date: text(r, "date")?, equity: maybe(r, "equity")?, net_deposits: maybe(r, "net_deposits")? }))
    })?;
    for (account, row) in rows {
        if account.is_empty() {
            together.push(row);
        } else {
            by_account.entry(account).or_default().push(row);
        }
    }
    Ok((together, by_account))
}

/// Each security's weights, from the JSON text they are kept as; unreadable is none.
pub fn exposures(conn: &Connection) -> Result<Exposures> {
    let weights = |raw: String| -> Value { serde_json::from_str(&raw).unwrap_or(Value::Null) };
    let rows = all(conn, "SELECT * FROM exposures", |r| {
        Ok((text(r, "key")?, Exposure::from_weights(&weights(text(r, "sectors")?), &weights(text(r, "countries")?))))
    })?;
    Ok(rows.into_iter().collect())
}

pub fn watchlist(conn: &Connection) -> Result<Vec<WatchRow>> {
    all(conn, "SELECT * FROM watchlist ORDER BY added_at, symbol", |r| {
        Ok(WatchRow { symbol: text(r, "symbol")?, exchange: text(r, "exchange")?, name: text(r, "name")?, currency: text(r, "currency")? })
    })
}

/// An item with no kind is a story, which is what a row stored before releases
/// were told apart is.
pub fn news(conn: &Connection) -> Result<Vec<NewsRow>> {
    all(conn, "SELECT * FROM news ORDER BY published_at DESC, id", |r| {
        let kind = text(r, "kind")?;
        Ok(NewsRow {
            id: text(r, "id")?,
            symbol: text(r, "symbol")?,
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

/// Symbol to the public record, newest first.
pub fn distributions(conn: &Connection) -> Result<HashMap<String, Vec<Distribution>>> {
    let rows = all(conn, "SELECT * FROM distributions ORDER BY symbol, ex_date DESC", |r| {
        Ok((r.get::<_, String>("symbol")?, Distribution { ex_date: text(r, "ex_date")?, pay_date: text(r, "pay_date")?, amount: number(r, "amount")? }))
    })?;
    let mut out: HashMap<String, Vec<Distribution>> = HashMap::new();
    for (symbol, row) in rows {
        out.entry(symbol).or_default().push(row);
    }
    Ok(out)
}

pub fn quotes(conn: &Connection) -> Result<Quotes> {
    let rows = all(conn, "SELECT * FROM quotes", |r| {
        Ok((
            r.get::<_, String>("symbol")?,
            Quote {
                price: maybe(r, "price")?,
                price_change: maybe(r, "price_change")?,
                percent_change: maybe(r, "percent_change")?,
                source: text(r, "source")?,
                ex_dividend_date: text(r, "ex_dividend_date")?,
            },
        ))
    })?;
    Ok(rows.into_iter().collect())
}

fn series(conn: &Connection, sql: &str, key: &str) -> Result<BTreeMap<String, f64>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?;
    rows.collect()
}

/// The pair's rate by day.
pub fn fx(conn: &Connection, pair: &str) -> Result<Fx> {
    Ok(series(conn, "SELECT date, rate FROM fx_rates WHERE pair = ? ORDER BY date", pair)?.into_iter().collect())
}

/// An index's close by day.
pub fn benchmark(conn: &Connection, symbol: &str) -> Result<BTreeMap<String, f64>> {
    series(conn, "SELECT date, close FROM benchmark_prices WHERE symbol = ? ORDER BY date", symbol)
}

pub fn groups(conn: &Connection) -> Result<Vec<TradeGroup>> {
    Ok(bagholder_model::lenient::rows(&Value::Array(crate::snapshot::groups_part(conn)?)))
}

pub fn journal(conn: &Connection) -> Result<Journal> {
    Ok(bagholder_model::input::journal_from(&crate::snapshot::journal(conn)?))
}

/// Never saved is `None`, which is not the same as saved empty.
pub fn tiles(conn: &Connection) -> Result<Option<Vec<TileRef>>> {
    let saved = crate::snapshot::tiles_part(conn)?;
    Ok((!saved.is_null()).then(|| bagholder_model::lenient::rows(&saved)))
}
