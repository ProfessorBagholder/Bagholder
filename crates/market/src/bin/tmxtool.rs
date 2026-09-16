//! The TMX readers that do not need the network, plus a live quote when asked,
//! so they can be compared with Python's.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::tmx as t;

    if mode == "live" {
        let path = doc.get("db").and_then(|v| v.as_str()).unwrap_or("");
        let conn = rusqlite::Connection::open(path).unwrap();
        bagholder_store::relabel::ensure(&conn).unwrap();
        let today = doc.get("today").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let out: Vec<Value> = doc.get("symbols").and_then(|v| v.as_array()).cloned().unwrap_or_default()
            .iter().map(|s| {
                let sym = s.get(0).and_then(|v| v.as_str()).unwrap_or("");
                let ex = s.get(1).and_then(|v| v.as_str()).unwrap_or("");
                let (q, d) = t::fetch_tmx(&conn, sym, ex, &today);
                json!({"symbol": sym, "quote": q, "dividends": d})
            }).collect();
        println!("{}", serde_json::to_string(&out).unwrap());
        return;
    }

    let quotes: Vec<Value> = doc.get("quotes").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|q| t::parse_tmx_quote(q).unwrap_or(Value::Null)).collect();
    let divs: Vec<Value> = doc.get("dividends").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|d| json!(t::parse_tmx_dividends(d))).collect();
    let venues: Vec<Value> = doc.get("venues").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|v| json!(t::tmx_venue(v.as_str().unwrap_or("")))).collect();
    let records: Vec<Value> = doc.get("records").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|r| json!(t::tmx_record_symbol(r.get(0).and_then(|v| v.as_str()).unwrap_or(""),
                                                   r.get(1).and_then(|v| v.as_str()).unwrap_or("")))).collect();
    let bares: Vec<Value> = doc.get("bares").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|b| json!(t::tmx_bare(b.as_str().unwrap_or("")))).collect();
    let canadian: Vec<Value> = doc.get("canadian").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|c| json!(t::is_canadian_listing(c.get(0).and_then(|v| v.as_str()).unwrap_or(""),
                                                     c.get(1).and_then(|v| v.as_str()).unwrap_or("")))).collect();
    println!("{}", serde_json::to_string(&json!({
        "quotes": quotes, "dividends": divs, "venues": venues,
        "records": records, "bares": bares, "canadian": canadian,
    })).unwrap());
}
