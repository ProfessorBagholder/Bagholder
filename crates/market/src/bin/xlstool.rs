//! The legacy Excel reader, on files given by path. It answers the table's
//! shape and its filled cells: a damaged file can name a cell tens of
//! thousands of columns out, and the dense grid of that is gigabytes of blanks.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::xls;
    let out: Vec<Value> = doc.get("files").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter()
        .map(|p| {
            let raw = match std::fs::read(p.as_str().unwrap_or("")) {
                Ok(r) => r,
                Err(e) => return json!({"ok": false, "error": e.to_string()}),
            };
            let found = match xls::streams(&raw) {
                Ok(f) => f,
                Err(e) => return json!({"ok": false, "error": e}),
            };
            let stream = match found.get("Workbook").or_else(|| found.get("Book")) {
                Some(s) if !s.is_empty() => s,
                _ => return json!({"ok": false, "error": "no workbook stream"}),
            };
            let grid = match xls::cells(stream) {
                Ok(g) => g,
                Err(e) => return json!({"ok": false, "error": e}),
            };
            if grid.is_empty() {
                return json!({"ok": true, "rows": 0, "cols": 0, "cells": []});
            }
            let rows = grid.keys().map(|k| k.0).max().unwrap() as usize + 1;
            let cols = grid.keys().map(|k| k.1).max().unwrap() as usize + 1;
            let mut keys: Vec<_> = grid.iter().filter(|(_, v)| **v != json!("")).collect();
            keys.sort_by_key(|(k, _)| **k);
            let filled: Vec<Value> = keys.into_iter().map(|((r, c), v)| json!([r, c, v])).collect();
            json!({"ok": true, "rows": rows, "cols": cols, "cells": filled})
        })
        .collect();
    println!("{}", serde_json::to_string(&out).unwrap());
}
