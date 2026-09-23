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

/// A field whose wire type is a list: null, absent or anything that is not an
/// array reads as empty; each element that is an object and reads as `T` is
/// kept, others dropped -- the same rule as `rows`.
pub fn list<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(rows(&Value::deserialize(d)?))
}

/// A nested object: null, absent, anything that is not an object, or an
/// object that fails to read as `T`, is `None`.
pub fn maybe_object<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(if v.is_object() { T::deserialize(&v).ok() } else { None })
}

/// Truthiness as Wealthsimple's own booleans are read: null or absent is
/// false; a number, string, array or object follows the general rule
/// (nonzero, non-empty); a bool is itself.
pub fn truthy<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Null => false,
        Value::Bool(b) => b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    })
}

/// The entries of an object whose value reads as `T`, sorted by key: a value
/// that is not an object, or does not read as `T`, drops its entry rather
/// than failing the whole map -- the same rule `rows` applies to a list.
pub fn objmap<T: serde::de::DeserializeOwned>(v: &Value) -> std::collections::BTreeMap<String, T> {
    match v {
        Value::Object(m) => m.iter().filter(|(_, x)| x.is_object()).filter_map(|(k, x)| Some((k.clone(), T::deserialize(x).ok()?))).collect(),
        _ => Default::default(),
    }
}

/// A field whose wire type is an object of objects: null, absent or anything
/// that is not an object reads as empty, as `list` does for an array.
pub fn map<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<std::collections::BTreeMap<String, T>, D::Error> {
    Ok(objmap(&Value::deserialize(d)?))
}

/// A field that is one row, many, or none: a bare object is one row, an
/// array many rows, anything else none.
pub fn one_or_many<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(match v {
        Value::Array(_) => rows(&v),
        Value::Object(_) => T::deserialize(&v).ok().into_iter().collect(),
        _ => vec![],
    })
}
