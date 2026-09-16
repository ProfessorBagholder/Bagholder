//! The history chain: the pure parts, and a live fetch when asked.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::history as h;

    if mode == "live" {
        let conn = rusqlite::Connection::open(doc.get("db").and_then(|v| v.as_str()).unwrap_or("")).unwrap();
        bagholder_store::relabel::ensure(&conn).unwrap();
        let today = doc.get("today").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let now = doc.get("now").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let stamp = doc.get("stamp").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let start = doc.get("start").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let end = doc.get("end").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let out: Vec<Value> = doc.get("records").and_then(|v| v.as_array()).cloned().unwrap_or_default()
            .iter().map(|rec| {
                let bars = h::ensure_history(&conn, rec, &start, &end, &today, now, &stamp).unwrap();
                json!({"symbol": rec.get("symbol"), "bars": bars})
            }).collect();
        println!("{}", serde_json::to_string(&out).unwrap());
        return;
    }

    let recs: Vec<Value> = doc.get("records").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let cands: Vec<Value> = recs.iter().map(|r| {
        json!(h::history_candidates(r).into_iter().map(|(s, k)| json!([s, k])).collect::<Vec<_>>())
    }).collect();
    let currencies: Vec<Value> = doc.get("barCurrency").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|c| json!(h::bar_currency(
            c.get(0).and_then(|v| v.as_str()).unwrap_or(""),
            c.get(1).and_then(|v| v.as_str()).unwrap_or(""),
            c.get(2).unwrap_or(&Value::Null)))).collect();
    let fx: BTreeMap<String, f64> = doc.get("fx").and_then(|v| v.as_object()).map(|m| {
        m.iter().filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f))).collect()
    }).unwrap_or_default();
    let converted: Vec<Value> = doc.get("convert").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|c| {
            let bars = c.get("bars").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            json!(h::in_position_currency_with(&bars,
                c.get("quoted").and_then(|v| v.as_str()).unwrap_or(""),
                c.get("currency").and_then(|v| v.as_str()).unwrap_or(""), &fx))
        }).collect();
    let agg: Vec<Value> = doc.get("aggregate").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|a| {
            let bars = a.get("bars").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            json!(h::aggregate_daily(&bars, a.get("tf").and_then(|v| v.as_str()).unwrap_or("1w")))
        }).collect();
    let charts: Vec<Value> = doc.get("yahooCharts").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|t| json!(bagholder_market::quotes::parse_yahoo_chart(t.as_str().unwrap_or("")))).collect();
    println!("{}", serde_json::to_string(&json!({
        "candidates": cands, "barCurrency": currencies, "convert": converted,
        "aggregate": agg, "yahooCharts": charts,
    })).unwrap());
}
