//! The canonical form of a record's payload.
//!
//! A source's reply is JSON. Two replies that say the same thing can differ in
//! the order of an object's keys, in whitespace, in how a string escapes its
//! characters and in how a number is written (`1.10`, `1.1`, `11e-1`). The book
//! keeps one form of each, so that "the same payload again" writes nothing and a
//! revision is only ever a change in what the source said:
//!
//! - object keys sorted (by their characters), no whitespace;
//! - strings with only `"`, `\` and control characters escaped;
//! - numbers as plain decimals with no exponent, no leading zeros, no trailing
//!   fractional zeros and no `-0`, worked out from the digits as written: a number
//!   never passes through a float, so no digit a source sent is lost.
//!
//! JSON that repeats a key within one object is refused: which value the source
//! meant cannot be known.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonError {
    pub at: usize,
    pub why: String,
}

impl fmt::Display for CanonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not JSON the book can keep, at byte {}: {}", self.at, self.why)
    }
}

impl std::error::Error for CanonError {}

/// The canonical form of `json`.
pub fn canonical(json: &str) -> Result<String, CanonError> {
    let mut p = Parser { s: json.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("text after the value"));
    }
    let mut out = String::with_capacity(json.len());
    write(&v, &mut out);
    Ok(out)
}

enum Value {
    Null,
    Bool(bool),
    /// Canonical decimal text.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

/// Deeper than this is not a reply any source sends, and would only exhaust the stack.
const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, why: &str) -> CanonError {
        CanonError { at: self.i, why: why.to_string() }
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn literal(&mut self, word: &[u8], v: Value) -> Result<Value, CanonError> {
        if self.s[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("not a JSON value"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, CanonError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.err("not a JSON value")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, CanonError> {
        self.i += 1;
        let mut map = BTreeMap::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("an object key must be a string"));
            }
            let at = self.i;
            let key = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return Err(self.err("expected ':'"));
            }
            self.i += 1;
            self.ws();
            let v = self.value(depth + 1)?;
            if map.insert(key.clone(), v).is_some() {
                return Err(CanonError { at, why: format!("the key {key:?} appears twice in one object") });
            }
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Object(map));
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, CanonError> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, CanonError> {
        let h = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("a short \\u escape"))?;
        let t = std::str::from_utf8(h).map_err(|_| self.err("a malformed \\u escape"))?;
        let n = u32::from_str_radix(t, 16).map_err(|_| self.err("a malformed \\u escape"))?;
        self.i += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, CanonError> {
        self.i += 1; // the opening quote
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.s.len() && !matches!(self.s[self.i], b'"' | b'\\') && self.s[self.i] >= 0x20 {
                self.i += 1;
            }
            let run = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| CanonError { at: start, why: "text that is not UTF-8".into() })?;
            out.push_str(run);
            match self.peek() {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or_else(|| self.err("an unfinished escape"))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                // a surrogate pair: the low half must follow
                                if !self.s[self.i..].starts_with(b"\\u") {
                                    return Err(self.err("a lone high surrogate"));
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("a high surrogate without its low half"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.err("a lone low surrogate"));
                            } else {
                                hi
                            };
                            out.push(char::from_u32(code).ok_or_else(|| self.err("not a character"))?);
                        }
                        _ => return Err(self.err("an unknown escape")),
                    }
                }
                Some(_) => return Err(self.err("a control character inside a string")),
                None => return Err(self.err("an unfinished string")),
            }
        }
    }

    /// A JSON number, as canonical decimal text worked out from its digits.
    fn number(&mut self) -> Result<Value, CanonError> {
        let start = self.i;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.i += 1;
        }
        let digits = |p: &mut Parser| {
            let from = p.i;
            while p.peek().is_some_and(|b| b.is_ascii_digit()) {
                p.i += 1;
            }
            from..p.i
        };
        let int = digits(self);
        if int.is_empty() {
            return Err(self.err("a number without digits"));
        }
        if self.s[int.start] == b'0' && int.len() > 1 {
            return Err(CanonError { at: int.start, why: "a number with a leading zero".into() });
        }
        let mut frac = 0..0;
        if self.peek() == Some(b'.') {
            self.i += 1;
            frac = digits(self);
            if frac.is_empty() {
                return Err(self.err("a number with no digits after its point"));
            }
        }
        let mut exp: i64 = 0;
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            let neg_exp = match self.peek() {
                Some(b'-') => {
                    self.i += 1;
                    true
                }
                Some(b'+') => {
                    self.i += 1;
                    false
                }
                _ => false,
            };
            let e = digits(self);
            if e.is_empty() {
                return Err(self.err("an exponent without digits"));
            }
            let text = std::str::from_utf8(&self.s[e]).unwrap_or("");
            // an exponent this large is not a quantity any source states
            let n: i64 = text.parse().ok().filter(|n: &i64| *n <= 10_000).ok_or_else(|| CanonError { at: start, why: "an exponent too large".into() })?;
            exp = if neg_exp { -n } else { n };
        }
        // all the digits, and where the point falls among them
        let mut all: Vec<u8> = self.s[int.clone()].to_vec();
        all.extend_from_slice(&self.s[frac.clone()]);
        let point = int.len() as i64 + exp; // digits before the point
        Ok(Value::Number(plain_decimal(negative, &all, point)))
    }
}

/// `digits` with the point after the first `point` of them (which may be before
/// the first or past the last), as canonical decimal text.
fn plain_decimal(negative: bool, digits: &[u8], point: i64) -> String {
    let first = digits.iter().position(|&d| d != b'0');
    let Some(first) = first else { return "0".to_string() };
    let last = digits.iter().rposition(|&d| d != b'0').unwrap_or(first);
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    // the significant digits run from `first` to `last`; place the point
    let (first, last) = (first as i64, last as i64);
    if point <= first {
        // 0.000ddd
        out.push_str("0.");
        for _ in 0..(first - point) {
            out.push('0');
        }
        out.extend(digits[first as usize..=last as usize].iter().map(|&b| b as char));
    } else if point > last {
        // ddd000
        out.extend(digits[first as usize..=last as usize].iter().map(|&b| b as char));
        for _ in 0..(point - last - 1) {
            out.push('0');
        }
    } else {
        out.extend(digits[first as usize..point as usize].iter().map(|&b| b as char));
        out.push('.');
        out.extend(digits[point as usize..=last as usize].iter().map(|&b| b as char));
    }
    out
}

fn write(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(n),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write(item, out);
            }
            out.push('}');
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> String {
        canonical(s).unwrap()
    }

    #[test]
    fn the_same_content_is_the_same_text() {
        let a = c(r#"{ "b": 1, "a": [true, null, "x"] }"#);
        let b = c("{\"a\":[true,null,\"x\"],\"b\":1}");
        assert_eq!(a, b);
        assert_eq!(a, r#"{"a":[true,null,"x"],"b":1}"#);
    }

    #[test]
    fn numbers_are_canonical_decimals_from_their_digits() {
        for (input, out) in [
            ("0", "0"),
            ("-0", "0"),
            ("-0.0e5", "0"),
            ("1.10", "1.1"),
            ("11e-1", "1.1"),
            ("1.1E0", "1.1"),
            ("1e3", "1000"),
            ("1.5e+2", "150"),
            ("-12.50", "-12.5"),
            ("0.000123", "0.000123"),
            ("123e-8", "0.00000123"),
            ("100", "100"),
            ("1050.00", "1050"),
            // more digits than any float or 96-bit decimal holds, kept whole
            ("3.14159265358979323846264338327950288419716939937510", "3.1415926535897932384626433832795028841971693993751"),
            ("12345678901234567890123456789012345678901234567890", "12345678901234567890123456789012345678901234567890"),
        ] {
            assert_eq!(c(input), out, "{input}");
        }
        for bad in ["01", "1.", ".5", "-", "1e", "1e+", "+1", "0x10", "1e100000"] {
            assert!(canonical(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn strings_keep_their_characters_with_minimal_escapes() {
        assert_eq!(c(r#""é\/\"\\\t""#), "\"é/\\\"\\\\\\u0009\"");
        assert_eq!(c(r#""🚀 Trading""#), "\"🚀 Trading\"");
        assert_eq!(c("\"🚀 Trading\""), "\"🚀 Trading\"");
        for bad in [r#""\ud83d""#, r#""\ude80""#, r#""\x""#, "\"a\u{1}b\"", "\"open"] {
            assert!(canonical(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_repeated_key_is_refused() {
        let e = canonical(r#"{"a":1,"a":2}"#).unwrap_err();
        assert!(e.why.contains("appears twice"), "{e}");
    }

    #[test]
    fn anything_but_one_json_value_is_refused() {
        for bad in ["", "{", "[1,]", "{\"a\" 1}", "{} {}", "nul", "tru", "[1 2]", "{1:2}"] {
            assert!(canonical(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(200) + &"]".repeat(200);
        assert!(canonical(&deep).is_err());
    }
}
