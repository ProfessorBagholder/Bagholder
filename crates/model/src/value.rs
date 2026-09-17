//! The loose-typed helpers the Python model leans on: activity rows arrive as
//! JSON maps whose fields may be absent, null, numeric or string, and every
//! reader has to agree on what that means. `_s` and `_num` in `model.py`.

use serde_json::Value;

pub const EPS: f64 = 1e-10;

/// `model._s`: absent or null reads as the empty string, everything else as
/// its text. A number keeps the shortest form that round-trips, because the
/// Python side stringifies ints without a trailing `.0`.
pub fn s(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(t)) => t.clone(),
        Some(Value::Bool(b)) => if *b { "True".into() } else { "False".into() },
        Some(Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                if f.fract() == 0.0 && f.abs() < 1e16 { format!("{:.1}", f) } else { f.to_string() }
            }
        }
        Some(other) => other.to_string(),
    }
}

/// `model._num`: null, the empty string, a non-number and NaN all read as the
/// default rather than raising, so one malformed row cannot empty a book.
pub fn num(v: Option<&Value>, default: f64) -> f64 {
    let f = match v {
        None | Some(Value::Null) => return default,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(t)) => {
            if t.is_empty() { return default; }
            match t.trim().parse::<f64>() { Ok(f) => f, Err(_) => return default }
        }
        Some(Value::Bool(b)) => if *b { 1.0 } else { 0.0 },
        Some(_) => return default,
    };
    if f.is_nan() { default } else { f }
}

/// Field of a JSON object by name, for rows that are always maps in practice
/// but must not panic when they are not.
pub fn get<'a>(row: &'a Value, key: &str) -> Option<&'a Value> {
    row.get(key).filter(|v| !v.is_null())
}

pub fn field_s(row: &Value, key: &str) -> String { s(get(row, key)) }
pub fn field_num(row: &Value, key: &str) -> f64 { num(get(row, key), 0.0) }

/// `model.compact`: upper-cased with whitespace, underscores and hyphens
/// removed. Symbols and account names run through this in the inner loops of
/// the FIFO match.
pub fn compact(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.trim().chars() {
        if c.is_whitespace() || c == '_' || c == '-' { continue; }
        out.extend(c.to_uppercase());
    }
    out
}

/// The separators Wealthsimple nicknames mix: ASCII space, the Unicode spaces
/// and the non-breaking hyphen `model._SPACE_RE` folds to one space.
pub fn is_space_like(c: char) -> bool {
    c.is_whitespace()
        || matches!(c, '\u{00a0}' | '\u{2000}'..='\u{200b}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{2011}')
}

/// `model.norm_account_name`: runs of space-like characters folded to one
/// ASCII space and trimmed, so equality filters on a nickname match.
pub fn norm_account_name(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut pending = false;
    for c in v.chars() {
        if is_space_like(c) { pending = true; continue; }
        if pending && !out.is_empty() { out.push(' '); }
        pending = false;
        out.push(c);
    }
    out.trim().to_string()
}

/// The same folding, but keeping case, for the option-symbol readers that
/// match on an upper-cased single-spaced form.
pub fn fold_spaces_upper(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut pending = false;
    for c in v.trim().chars() {
        if is_space_like(c) { pending = true; continue; }
        if pending && !out.is_empty() { out.push(' '); }
        pending = false;
        out.extend(c.to_uppercase());
    }
    out
}

pub fn fmt8(v: f64) -> String { format!("{:.8}", v) }

/// A float written the way Python writes one, for the derived rows whose
/// descriptions are built by string formatting: `200.0`, not `200`.
pub fn num_repr(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e16 { format!("{:.1}", v) } else { v.to_string() }
}

/// Python's `sum` over floats: the integer 0 when there is nothing to add,
/// else the float total from +0.0 (never -0.0).
pub fn sum_of(empty: bool, total: f64) -> serde_json::Value {
    if empty { serde_json::json!(0) } else { serde_json::json!(total + 0.0) }
}
