//! The canonical form of a record's payload.
//!
//! A source's reply is JSON. Two replies that say the same thing can differ in
//! the order of an object's keys, in whitespace, in how a string escapes its
//! characters and in how a number is written (`1.10`, `1.1`, `11e-1`). The book
//! keeps one form of each, so that "the same payload again" writes nothing and a
//! revision is only ever a change in what the source said. The form, and the
//! exact reading behind it, are `bagholder_core::json`'s.

use bagholder_core::json;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonError(pub json::JsonError);

impl fmt::Display for CanonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not JSON the book can keep, at byte {}: {}", self.0.at, self.0.why)
    }
}

impl std::error::Error for CanonError {}

/// The canonical form of `json`.
pub fn canonical(text: &str) -> Result<String, CanonError> {
    json::parse(text).map(|v| v.canonical()).map_err(CanonError)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_is_kept_in_one_form() {
        assert_eq!(canonical(r#"{ "b": 1.10, "a": [true, null, "x"] }"#).unwrap(), r#"{"a":[true,null,"x"],"b":1.1}"#);
        let e = canonical(r#"{"a":1,"a":2}"#).unwrap_err();
        assert!(e.to_string().starts_with("not JSON the book can keep"), "{e}");
    }
}
