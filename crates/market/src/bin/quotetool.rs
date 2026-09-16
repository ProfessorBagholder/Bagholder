//! The quote routing and parsers, and a live refresh when asked.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::quotes as q;

    if mode == "live" {
        let conn = rusqlite::Connection::open(doc.get("db").and_then(|v| v.as_str()).unwrap_or("")).unwrap();
        bagholder_store::relabel::ensure(&conn).unwrap();
        let today = doc.get("today").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let now = doc.get("now").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let stamp = doc.get("stamp").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let syms: Vec<Value> = doc.get("symbols").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let n = q::refresh_quotes(&conn, &syms, &today, now, &stamp).unwrap();
        println!("{}", serde_json::to_string(&json!({
            "written": n,
            "quotes": bagholder_store::market::quotes(&conn).unwrap(),
        })).unwrap());
        return;
    }

    let recs: Vec<Value> = doc.get("records").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let sources: Vec<Value> = recs.iter().map(|r| match q::quote_source(r) {
        Some((s, k)) => json!([s, k]),
        None => Value::Null,
    }).collect();
    let forms: Vec<Value> = recs.iter().map(|r| json!(q::yahoo_forms(r))).collect();
    let yq: Vec<Value> = doc.get("yahoo").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|t| q::parse_yahoo_quote(t.as_str().unwrap_or("")).unwrap_or(Value::Null)).collect();
    let roots: Vec<Value> = doc.get("roots").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|s| json!(q::yahoo_root(s.as_str().unwrap_or("")))).collect();
    let tqs: Vec<Value> = doc.get("tmxQuoteSymbols").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|t| json!(q::tmx_quote_symbol(
            t.get(0).and_then(|v| v.as_str()).unwrap_or(""),
            t.get(1).and_then(|v| v.as_str()).unwrap_or(""),
            t.get(2).and_then(|v| v.as_str()).unwrap_or("")))).collect();
    println!("{}", serde_json::to_string(&json!({
        "sources": sources, "forms": forms, "yahoo": yq, "roots": roots, "tmxQuoteSymbols": tqs,
    })).unwrap());
}
