//! Reading a database an earlier version of Bagholder kept, from a copy.
//!
//! The earlier app stored money as binary floats. Each is read as the shortest
//! decimal that reads back to the same float, which is the decimal Wealthsimple
//! sent wherever the app stored what it was sent; values the app worked out
//! itself (a per-unit price, a booked fill's cash) are read the same way and the
//! import mapping says which of them it carries. This is the one place in the
//! new code where a float becomes a decimal (`bagholder-core/tests/boundaries.rs`).

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

use crate::{BookError, Result};

/// A copy of the database at `src`, made at `dst` with SQLite's backup API from a
/// read-only connection: consistent even while the earlier app is writing, and
/// the original is not written.
pub fn copy_database(src: &Path, dst: &Path) -> Result<()> {
    let from = Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let mut to = Connection::open(dst)?;
    {
        let backup = rusqlite::backup::Backup::new(&from, &mut to)?;
        backup.run_to_completion(256, std::time::Duration::from_millis(0), None)?;
    }
    // the copy is one file, read and then deleted: reading it in rollback mode
    // leaves no write-ahead log or shared-memory file beside it
    let _: String = to.query_row("PRAGMA journal_mode = DELETE", [], |r| r.get(0))?;
    Ok(())
}

/// What the import reads from the earlier database.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OldDatabase {
    pub schema_version: String,
    pub accounts: Vec<OldAccount>,
    pub securities: BTreeMap<String, OldSecurity>,
    pub activities: Vec<OldActivity>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OldAccount {
    pub id: String,
    pub nickname: Option<String>,
    pub unified_account_type: Option<String>,
    pub currency: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OldSecurity {
    pub id: String,
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub primary_exchange: Option<String>,
    pub primary_mic: Option<String>,
    pub currency: Option<String>,
    pub underlying_id: Option<String>,
}

/// An activity row as the earlier app stored it: every column, text as stored
/// and numbers as decimal text.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OldActivity {
    pub id: String,
    pub canonical_id: Option<String>,
    pub occurred_at: Option<String>,
    pub transaction_date: Option<String>,
    pub settlement_date: Option<String>,
    pub account_id: Option<String>,
    pub book_id: Option<String>,
    pub fifo_id: Option<String>,
    pub account_type: Option<String>,
    pub activity_type: Option<String>,
    pub activity_sub_type: Option<String>,
    pub description: Option<String>,
    pub direction: Option<String>,
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub currency: Option<String>,
    pub quantity: Option<String>,
    pub unit_price: Option<String>,
    pub commission: Option<String>,
    pub net_cash_amount: Option<String>,
    pub category: Option<String>,
    pub balance: Option<String>,
    pub source: Option<String>,
    pub raw_type: Option<String>,
    pub aft_type: Option<String>,
    pub counter_symbol: Option<String>,
    pub security_id: Option<String>,
}

/// A stored value as text: text as it is, an integer in decimal, a float as the
/// shortest decimal that reads back to it. A float that is not a number is kept
/// as its name (`NaN`), which the mapping refuses to read as an amount. Text that
/// is not UTF-8, or a blob, is not something the earlier app wrote: an error
/// naming where it is.
fn text(v: ValueRef, table: &str, column: &str, row: &str) -> rusqlite::Result<Option<String>> {
    let bad = |why: &str| rusqlite::Error::InvalidColumnType(0, format!("{table}.{column} of row {row:?}: {why}"), rusqlite::types::Type::Text);
    Ok(match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(float_text(f)),
        ValueRef::Text(t) => Some(std::str::from_utf8(t).map_err(|_| bad("text that is not UTF-8"))?.to_string()),
        ValueRef::Blob(_) => return Err(bad("a blob where text or a number was stored")),
    })
}

/// A row's own id: present and not empty, or the row cannot be told from others.
fn id(v: ValueRef, table: &str) -> rusqlite::Result<String> {
    match text(v, table, "id", "?")? {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => Err(rusqlite::Error::InvalidColumnType(0, format!("{table}: a row with no id"), rusqlite::types::Type::Null)),
    }
}

/// Rust prints a float as the shortest decimal that reads back to the same
/// float, and never with an exponent.
fn float_text(f: f64) -> String {
    if !f.is_finite() {
        return f.to_string();
    }
    let s = format!("{f}");
    if s == "-0" { "0".into() } else { s }
}

/// Read what the import needs from the copy at `path`.
pub fn read(path: &Path) -> Result<OldDatabase> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let meta = |key: &str| -> Result<Option<String>> {
        let mut stmt = conn.prepare("SELECT value FROM meta WHERE key = ?")?;
        let mut rows = stmt.query([key])?;
        Ok(match rows.next()? {
            Some(r) => text(r.get_ref(0)?, "meta", "value", key)?,
            None => None,
        })
    };
    let schema_version = meta("schema_version")?.unwrap_or_default();
    if schema_version != "13" {
        return Err(BookError::Refused(format!(
            "the database is schema {schema_version:?}; the import reads schema 13, which every earlier version since the first release wrote on start"
        )));
    }

    let accounts = rows(&conn, "SELECT id, nickname, unified_account_type, currency, status, type FROM accounts ORDER BY id", |r| {
        let id = id(r.get_ref(0)?, "accounts")?;
        let t = |i: usize, c: &str| text(r.get_ref(i)?, "accounts", c, &id);
        Ok(OldAccount { nickname: t(1, "nickname")?, unified_account_type: t(2, "unified_account_type")?, currency: t(3, "currency")?, status: t(4, "status")?, kind: t(5, "type")?, id })
    })?;
    let securities = rows(&conn, "SELECT id, symbol, name, primary_exchange, primary_mic, currency, underlying_id FROM securities ORDER BY id", |r| {
        let id = id(r.get_ref(0)?, "securities")?;
        let t = |i: usize, c: &str| text(r.get_ref(i)?, "securities", c, &id);
        Ok(OldSecurity {
            symbol: t(1, "symbol")?,
            name: t(2, "name")?,
            primary_exchange: t(3, "primary_exchange")?,
            primary_mic: t(4, "primary_mic")?,
            currency: t(5, "currency")?,
            underlying_id: t(6, "underlying_id")?,
            id,
        })
    })?
    .into_iter()
    .map(|s| (s.id.clone(), s))
    .collect();
    let activities = rows(
        &conn,
        "SELECT id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, account_type, activity_type,
                activity_sub_type, description, direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount,
                category, balance, source, raw_type, aft_type, counter_symbol, security_id
         FROM activities ORDER BY transaction_date, occurred_at, id",
        |r| {
            let row_id = id(r.get_ref(0)?, "activities")?;
            let t = |i: usize, c: &str| text(r.get_ref(i)?, "activities", c, &row_id);
            Ok(OldActivity {
                canonical_id: t(1, "canonical_id")?,
                occurred_at: t(2, "occurred_at")?,
                transaction_date: t(3, "transaction_date")?,
                settlement_date: t(4, "settlement_date")?,
                account_id: t(5, "account_id")?,
                book_id: t(6, "book_id")?,
                fifo_id: t(7, "fifo_id")?,
                account_type: t(8, "account_type")?,
                activity_type: t(9, "activity_type")?,
                activity_sub_type: t(10, "activity_sub_type")?,
                description: t(11, "description")?,
                direction: t(12, "direction")?,
                symbol: t(13, "symbol")?,
                name: t(14, "name")?,
                currency: t(15, "currency")?,
                quantity: t(16, "quantity")?,
                unit_price: t(17, "unit_price")?,
                commission: t(18, "commission")?,
                net_cash_amount: t(19, "net_cash_amount")?,
                category: t(20, "category")?,
                balance: t(21, "balance")?,
                source: t(22, "source")?,
                raw_type: t(23, "raw_type")?,
                aft_type: t(24, "aft_type")?,
                counter_symbol: t(25, "counter_symbol")?,
                security_id: t(26, "security_id")?,
                id: row_id,
            })
        },
    )?;

    Ok(OldDatabase {
        schema_version,
        accounts,
        securities,
        activities,
    })
}

fn rows<T>(conn: &Connection, sql: &str, f: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let out = stmt.query_map([], f)?.collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_float_reads_as_its_shortest_decimal() {
        assert_eq!(float_text(1050.0), "1050");
        assert_eq!(float_text(0.321720925242232), "0.321720925242232");
        assert_eq!(float_text(-44.47), "-44.47");
        assert_eq!(float_text(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(float_text(1e-7), "0.0000001");
        assert_eq!(float_text(-0.0), "0");
        assert_eq!(float_text(f64::NAN), "NaN");
    }
}
