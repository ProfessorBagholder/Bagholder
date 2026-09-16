//! The universe readers, on fixtures or live.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::universes as u;

    if mode == "live" {
        println!("{}", serde_json::to_string(&json!({
            "screener": u::fetch_screener(),
            "canada": u::fetch_canada(),
        })).unwrap());
        return;
    }
    let arr = |k: &str| doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let screener: Vec<Value> = arr("screener").iter().map(|d| json!(u::parse_screener(d))).collect();
    let us: Vec<Value> = arr("rows").iter().map(|d| {
        let rows = d.as_array().cloned().unwrap_or_default();
        json!(u::us_rows(&rows, u::TOP))
    }).collect();
    let intl: Vec<Value> = arr("rows").iter().map(|d| {
        let rows = d.as_array().cloned().unwrap_or_default();
        json!(u::intl_rows(&rows, u::TOP))
    }).collect();
    let cons: Vec<Value> = arr("constituents").iter().map(|d| json!(u::parse_constituents(d))).collect();
    let tiles: Vec<Value> = arr("tiles").iter().map(|d| json!(u::parse_tile_quote(d))).collect();
    let sectors: Vec<Value> = arr("sectors").iter()
        .map(|s| json!(u::sector_of(s.as_str().unwrap_or("")))).collect();
    println!("{}", serde_json::to_string(&json!({
        "screener": screener, "us": us, "intl": intl,
        "constituents": cons, "tiles": tiles, "sectors": sectors,
    })).unwrap());
}
