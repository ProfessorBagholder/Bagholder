//! Reading rows that are not strict about their types.
//!
//! The store writes clean rows, but the model is also fed by the shared cases
//! (`tests/cases`, read by every implementation), by CSV imports and by rows
//! entered by hand, where a quantity may be `100` or `"100"`, a field may be
//! null or missing, and an id may be a number. The rule is one rule, applied as
//! each row is read and nowhere after: text is `value::s`, a number is
//! `value::num`. Past this module every field has its type.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::value::{num, s};

/// Absent or null is the empty string; a number or a bool is its text.
pub fn text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(s(Some(&Value::deserialize(d)?)))
}

/// Absent, null, the empty string, anything unreadable and NaN are zero; a
/// numeric string is its number.
pub fn number<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(num(Some(&Value::deserialize(d)?), 0.0))
}

/// As `number`, but what is not there stays not there.
pub fn maybe_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(opt_num(&Value::deserialize(d)?))
}

/// A number if the value is one or spells one.
pub fn opt_num(v: &Value) -> Option<f64> {
    match v {
        Value::Null => None,
        Value::String(t) if t.trim().is_empty() => None,
        other => {
            let f = num(Some(other), f64::NAN);
            if f.is_nan() { None } else { Some(f) }
        }
    }
}

/// A list of text, whatever its items are; anything that is not a list is empty.
pub fn texts<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(xs) => xs.iter().map(|x| s(Some(x))).collect(),
        _ => vec![],
    })
}

/// The rows of a list that are objects, each read as `T`; a row that cannot be
/// read is left out rather than failing the list, so one malformed row cannot
/// empty a book.
pub fn rows<T: serde::de::DeserializeOwned>(v: &Value) -> Vec<T> {
    match v {
        Value::Array(xs) => xs.iter().filter(|x| x.is_object()).filter_map(|x| T::deserialize(x).ok()).collect(),
        _ => vec![],
    }
}
