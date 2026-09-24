//! Reading a reply: exact and strict (`docs/plans/stage-3a-sources.md`, "Reading
//! a reply").
//!
//! A reply is parsed by `bagholder_core::json`, so every number is the decimal
//! its digits spell. An adapter then reads the fields it needs through a
//! [`Node`]:
//!
//! - a required accessor (`text`, `dec`, `day`, `list`, …) answers the field or
//!   a [`Mismatch`] naming its path: absent, null, or of another type;
//! - an optional accessor (`opt_text`, `opt_day`, …) is for a field real replies
//!   show null: null reads as "the source states none", and an absent key is
//!   still a mismatch.
//!
//! A field the adapter does not read is ignored. What a reply carries beyond
//! that is its [`shape`], compared against the recorded replies' by
//! [`shape_change`].

use bagholder_core::jiff::civil::Date;
use bagholder_core::json::{self, Value};
use bagholder_core::{Dec, DecError};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A reply that does not have the shape the adapter reads: the path of the
/// field, and what was wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub path: String,
    pub why: String,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "the reply: {}", self.why)
        } else {
            write!(f, "{}: {}", self.path, self.why)
        }
    }
}

impl std::error::Error for Mismatch {}

pub type Read<T> = Result<T, Mismatch>;

/// The whole of a reply's text as one JSON value, or a mismatch at its root.
pub fn parse(text: &str) -> Read<Value> {
    json::parse(text).map_err(|e| Mismatch { path: String::new(), why: format!("not JSON: {} (at byte {})", e.why, e.at) })
}

/// One value of a reply and where it sits in it.
#[derive(Clone, Debug)]
pub struct Node<'a> {
    value: &'a Value,
    path: String,
}

impl<'a> Node<'a> {
    pub fn root(value: &'a Value) -> Node<'a> {
        Node { value, path: String::new() }
    }

    pub fn value(&self) -> &'a Value {
        self.value
    }

    /// The path of this value: `dividends[3].amount`.
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn mismatch(&self, why: impl Into<String>) -> Mismatch {
        Mismatch { path: self.path.clone(), why: why.into() }
    }

    fn child_path(&self, key: &str) -> String {
        if self.path.is_empty() { key.to_string() } else { format!("{}.{key}", self.path) }
    }

    /// The field `key` of this object, present (it may be null).
    pub fn field(&self, key: &str) -> Read<Node<'a>> {
        let Value::Object(map) = self.value else {
            return Err(self.mismatch(format!("expected an object, found {}", self.value.kind())));
        };
        match map.get(key) {
            Some(v) => Ok(Node { value: v, path: self.child_path(key) }),
            None => Err(Mismatch { path: self.child_path(key), why: "absent".into() }),
        }
    }

    /// The field `key`, or `None` where it is null.
    fn present(&self, key: &str) -> Read<Option<Node<'a>>> {
        let n = self.field(key)?;
        Ok(if matches!(n.value, Value::Null) { None } else { Some(n) })
    }

    // -- this value, read as one type ------------------------------------

    pub fn as_text(&self) -> Read<&'a str> {
        match self.value {
            Value::String(s) => Ok(s.as_str()),
            v => Err(self.mismatch(format!("expected text, found {}", v.kind()))),
        }
    }

    /// A JSON number, read exactly as a decimal.
    pub fn as_dec(&self) -> Read<Dec> {
        match self.value {
            Value::Number(n) => dec_from(n).map_err(|why| self.mismatch(why)),
            v => Err(self.mismatch(format!("expected a decimal, found {}", v.kind()))),
        }
    }

    /// A decimal written as text (`"1.3665"`), read exactly.
    pub fn as_dec_text(&self) -> Read<Dec> {
        match self.value {
            Value::String(t) => dec_from(t).map_err(|why| self.mismatch(why)),
            v => Err(self.mismatch(format!("expected a decimal written as text, found {}", v.kind()))),
        }
    }

    /// A whole number.
    pub fn as_int(&self) -> Read<i64> {
        match self.value {
            Value::Number(n) => n.parse::<i64>().map_err(|_| self.mismatch(format!("expected a whole number, found {n}"))),
            v => Err(self.mismatch(format!("expected a whole number, found {}", v.kind()))),
        }
    }

    pub fn as_bool(&self) -> Read<bool> {
        match self.value {
            Value::Bool(b) => Ok(*b),
            v => Err(self.mismatch(format!("expected a boolean, found {}", v.kind()))),
        }
    }

    /// A day written `YYYY-MM-DD`, and a real one.
    pub fn as_day(&self) -> Read<Date> {
        match self.value {
            Value::String(t) => day_from(t).map_err(|why| self.mismatch(why)),
            v => Err(self.mismatch(format!("expected a day, found {}", v.kind()))),
        }
    }

    /// The items of a list.
    pub fn as_list(&self) -> Read<Vec<Node<'a>>> {
        match self.value {
            Value::Array(items) => Ok(items.iter().enumerate().map(|(i, v)| Node { value: v, path: format!("{}[{i}]", self.path) }).collect()),
            v => Err(self.mismatch(format!("expected a list, found {}", v.kind()))),
        }
    }

    /// An object's keys, in order.
    pub fn keys(&self) -> Read<Vec<&'a str>> {
        match self.value {
            Value::Object(map) => Ok(map.keys().map(String::as_str).collect()),
            v => Err(self.mismatch(format!("expected an object, found {}", v.kind()))),
        }
    }

    // -- required fields -------------------------------------------------

    pub fn obj(&self, key: &str) -> Read<Node<'a>> {
        let n = self.field(key)?;
        match n.value {
            Value::Object(_) => Ok(n),
            v => Err(n.mismatch(format!("expected an object, found {}", v.kind()))),
        }
    }

    pub fn text(&self, key: &str) -> Read<&'a str> {
        self.field(key)?.as_text()
    }

    pub fn dec(&self, key: &str) -> Read<Dec> {
        self.field(key)?.as_dec()
    }

    pub fn dec_text(&self, key: &str) -> Read<Dec> {
        self.field(key)?.as_dec_text()
    }

    pub fn int(&self, key: &str) -> Read<i64> {
        self.field(key)?.as_int()
    }

    pub fn bool(&self, key: &str) -> Read<bool> {
        self.field(key)?.as_bool()
    }

    pub fn day(&self, key: &str) -> Read<Date> {
        self.field(key)?.as_day()
    }

    pub fn list(&self, key: &str) -> Read<Vec<Node<'a>>> {
        self.field(key)?.as_list()
    }

    // -- optional fields: null is "none stated", absent is still a mismatch --

    pub fn opt_text(&self, key: &str) -> Read<Option<&'a str>> {
        self.present(key)?.map(|n| n.as_text()).transpose()
    }

    pub fn opt_dec(&self, key: &str) -> Read<Option<Dec>> {
        self.present(key)?.map(|n| n.as_dec()).transpose()
    }

    pub fn opt_dec_text(&self, key: &str) -> Read<Option<Dec>> {
        self.present(key)?.map(|n| n.as_dec_text()).transpose()
    }

    pub fn opt_day(&self, key: &str) -> Read<Option<Date>> {
        self.present(key)?.map(|n| n.as_day()).transpose()
    }
}

/// A decimal from its canonical or written text: exact, or why not.
fn dec_from(text: &str) -> Result<Dec, String> {
    Dec::parse(text).map_err(|e| match e {
        DecError::Overflow => format!("{text} has more digits than a decimal holds"),
        _ => format!("expected a decimal, found {text:?}"),
    })
}

/// A day from `YYYY-MM-DD`: that form only, and a day that exists.
pub fn day_from(text: &str) -> Result<Date, String> {
    let b = text.as_bytes();
    let shaped = b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if !shaped {
        return Err(format!("expected a day (YYYY-MM-DD), found {text:?}"));
    }
    text.parse::<Date>().map_err(|_| format!("{text} is not a day"))
}

// -- the shape of a reply ------------------------------------------------------

/// Every path a reply carries, array indices folded (`observations[].d`), with the
/// kinds of value found there.
pub type Shape = BTreeMap<String, BTreeSet<&'static str>>;

pub fn shape(v: &Value) -> Shape {
    fn walk(v: &Value, path: &str, out: &mut Shape) {
        out.entry(path.to_string()).or_default().insert(v.kind());
        match v {
            Value::Object(map) => {
                for (k, item) in map {
                    let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                    walk(item, &p, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, &format!("{path}[]"), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Shape::new();
    walk(v, "", &mut out);
    out
}

/// The union of several replies' shapes: what the recorded replies carry.
pub fn union(shapes: impl IntoIterator<Item = Shape>) -> Shape {
    let mut out = Shape::new();
    for s in shapes {
        for (p, kinds) in s {
            out.entry(p).or_default().extend(kinds);
        }
    }
    out
}

/// How a reply's shape differs from the recorded ones.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShapeChange {
    /// Paths the recorded replies carry and this one does not, where it could
    /// have: the path's parent is here as an object, or as a list with items.
    pub gone: Vec<String>,
    /// Paths this reply carries that no recorded reply does.
    pub new: Vec<String>,
}

impl ShapeChange {
    pub fn is_empty(&self) -> bool {
        self.gone.is_empty() && self.new.is_empty()
    }
}

impl fmt::Display for ShapeChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if !self.gone.is_empty() {
            parts.push(format!("gone: {}", self.gone.join(", ")));
        }
        if !self.new.is_empty() {
            parts.push(format!("new: {}", self.new.join(", ")));
        }
        write!(f, "{}", parts.join("; "))
    }
}

/// `path`'s parent and whether the step to it is a list's item.
fn parent(path: &str) -> Option<(&str, bool)> {
    if let Some(p) = path.strip_suffix("[]") {
        return Some((p, true));
    }
    path.rfind('.').map(|i| (&path[..i], false)).or(if path.is_empty() { None } else { Some(("", false)) })
}

pub fn shape_change(recorded: &Shape, reply: &Shape) -> Option<ShapeChange> {
    let mut change = ShapeChange::default();
    for p in recorded.keys() {
        if reply.contains_key(p) {
            continue;
        }
        let Some((up, item)) = parent(p) else { continue };
        let could = match reply.get(up) {
            // a list's items are gone only if the list has items and they lack it
            Some(kinds) if item => kinds.contains("a list") && reply.keys().any(|k| k.starts_with(&format!("{up}[]"))),
            Some(kinds) => kinds.contains("an object"),
            None => false,
        };
        if could {
            change.gone.push(p.clone());
        }
    }
    for p in reply.keys() {
        if !recorded.contains_key(p) {
            change.new.push(p.clone());
        }
    }
    if change.is_empty() { None } else { Some(change) }
}
