//! The market cache's `meta` keys, and JSON as it is stored.

use rusqlite::{Connection, Result};
use serde_json::Value;

/// `get_meta`: the value, or `default` where the key has none. A store that
/// cannot be read is the failure, never the default.
pub fn get_meta(conn: &Connection, key: &str, default: &str) -> Result<String> {
    use rusqlite::OptionalExtension;
    let v: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?", [key], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(v.unwrap_or_else(|| default.to_string()))
}

/// `set_meta`.
pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Compact JSON with `", "` and `": "` separators, and every
/// non-ASCII character escaped.
///
/// One way of writing them, so the same content is always the same text where
/// it is stored (the market cache's `meta`, the book's settings).
pub fn json_text(v: &Value) -> String {
    let mut out = String::new();
    write_py_json(v, &mut out);
    out
}

fn write_py_json(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_py_str(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_py_json(x, out);
            }
            out.push(']');
        }
        Value::Object(m) => {
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_py_str(k, out);
                out.push_str(": ");
                write_py_json(x, out);
            }
            out.push('}');
        }
    }
}

fn write_py_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) < 0x7f => out.push(c),
            // ensure_ascii: anything above ASCII goes out as an escape, and
            // anything outside the basic plane as a surrogate pair.
            c => {
                let cp = c as u32;
                if cp > 0xffff {
                    let v = cp - 0x10000;
                    out.push_str(&format!("\\u{:04x}\\u{:04x}", 0xd800 + (v >> 10), 0xdc00 + (v & 0x3ff)));
                } else {
                    out.push_str(&format!("\\u{:04x}", cp));
                }
            }
        }
    }
    out.push('"');
}
