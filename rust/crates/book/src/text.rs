//! Reading stored text back into typed values, strictly: a value that does not
//! read is an error naming its table and column, never a default.

use bagholder_core::{Currency, Dec, Money};

use crate::{BookError, Result};

pub(crate) fn corrupt(table: &'static str, column: &'static str, value: &str, why: impl ToString) -> BookError {
    BookError::Corrupt { table, column, value: value.to_string(), why: why.to_string() }
}

/// A stored decimal: exactly the canonical text the book writes, or corrupt.
pub(crate) fn dec(table: &'static str, column: &'static str, v: &str) -> Result<Dec> {
    let d = Dec::parse(v).map_err(|e| corrupt(table, column, v, e))?;
    if d.to_text() != v {
        return Err(corrupt(table, column, v, "not in the form the book writes"));
    }
    Ok(d)
}

pub(crate) fn opt_dec(table: &'static str, column: &'static str, v: Option<String>) -> Result<Option<Dec>> {
    v.map(|s| dec(table, column, &s)).transpose()
}

pub(crate) fn currency(table: &'static str, column: &'static str, v: &str) -> Result<Currency> {
    Currency::parse(v).map_err(|e| corrupt(table, column, v, e))
}

pub(crate) fn money(table: &'static str, column: &'static str, amount: Option<String>, cur: Option<String>) -> Result<Option<Money>> {
    match (amount, cur) {
        (None, None) => Ok(None),
        (Some(a), Some(c)) => Ok(Some(Money::new(dec(table, column, &a)?, currency(table, column, &c)?))),
        (a, c) => Err(corrupt(table, column, &format!("{a:?} {c:?}"), "an amount without its currency")),
    }
}

pub(crate) fn date(table: &'static str, column: &'static str, v: &str) -> Result<jiff::civil::Date> {
    v.parse().map_err(|e| corrupt(table, column, v, e))
}

pub(crate) fn opt_date(table: &'static str, column: &'static str, v: Option<String>) -> Result<Option<jiff::civil::Date>> {
    v.map(|s| date(table, column, &s)).transpose()
}

pub(crate) fn instant(table: &'static str, column: &'static str, v: &str) -> Result<jiff::Timestamp> {
    v.parse().map_err(|e| corrupt(table, column, v, e))
}

pub(crate) fn opt_instant(table: &'static str, column: &'static str, v: Option<String>) -> Result<Option<jiff::Timestamp>> {
    v.map(|s| instant(table, column, &s)).transpose()
}

/// Any value with a strict `parse` (ids, words, names).
pub(crate) fn parsed<T, E: ToString>(table: &'static str, column: &'static str, v: &str, parse: impl FnOnce(&str) -> std::result::Result<T, E>) -> Result<T> {
    parse(v).map_err(|e| corrupt(table, column, v, e))
}

pub(crate) fn opt_parsed<T, E: ToString>(table: &'static str, column: &'static str, v: Option<String>, parse: impl FnOnce(&str) -> std::result::Result<T, E>) -> Result<Option<T>> {
    v.map(|s| parsed(table, column, &s, parse)).transpose()
}

/// A day as stored.
pub(crate) fn day(d: jiff::civil::Date) -> String {
    d.to_string()
}

/// An instant as stored: RFC 3339 in UTC.
pub(crate) fn at(t: jiff::Timestamp) -> String {
    t.to_string()
}
