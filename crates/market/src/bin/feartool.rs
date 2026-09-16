//! The fear-and-greed readers, on fixtures or live.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::fear as f;

    if mode == "live" {
        let out: Vec<Value> = doc.get("indexes").and_then(|v| v.as_array()).cloned().unwrap_or_default()
            .iter().map(|i| f::read(i.as_str().unwrap_or(""))).collect();
        println!("{}", serde_json::to_string(&out).unwrap());
        return;
    }
    let stocks: Vec<Value> = doc.get("stocks").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|d| f::parse_stocks(d)).collect();
    let crypto: Vec<Value> = doc.get("crypto").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|d| f::parse_crypto(d)).collect();
    let bands: Vec<Value> = doc.get("bands").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|s| json!(f::band(s.as_f64()))).collect();
    let ratings: Vec<Value> = doc.get("ratings").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|r| json!(f::rating(r.get(0).and_then(|v| v.as_str()).unwrap_or(""),
                                        r.get(1).and_then(|v| v.as_f64())))).collect();
    println!("{}", serde_json::to_string(&json!({
        "stocks": stocks, "crypto": crypto, "bands": bands, "ratings": ratings,
    })).unwrap());
}
