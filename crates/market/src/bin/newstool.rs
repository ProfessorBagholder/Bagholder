//! The news parsers, on fixtures or live.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_market::news as n;

    if mode == "live" {
        let db = doc.get("db").and_then(|v| v.as_str()).unwrap_or("");
        let conn = rusqlite::Connection::open(db).unwrap();
        let today = doc.get("today").and_then(|v| v.as_str()).unwrap_or("");
        let now = doc.get("now").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
        let out: Vec<Value> = doc.get("listings").and_then(|v| v.as_array()).cloned().unwrap_or_default()
            .iter()
            .map(|l| {
                let f = |i: usize| l.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let (src, rows) = n::fetch_symbol(&conn, &f(0), &f(1), &f(2), today, now);
                json!({"source": src, "rows": rows})
            })
            .collect();
        println!("{}", serde_json::to_string(&out).unwrap());
        return;
    }

    let now = doc.get("now").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
    let arr = |k: &str| doc.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let tmx: Vec<Value> = arr("tmx").iter()
        .map(|c| json!(n::parse_tmx_news(c.get("data").unwrap_or(&Value::Null),
                                         c.get("symbol").and_then(|v| v.as_str()).unwrap_or(""))))
        .collect();
    let nasdaq: Vec<Value> = arr("nasdaq").iter()
        .map(|c| json!(n::parse_nasdaq_news(
            c.get("data").unwrap_or(&Value::Null),
            c.get("now").and_then(|v| v.as_f64()).unwrap_or(now as f64) as i64,
            c.get("symbol").and_then(|v| v.as_str()).unwrap_or(""),
            c.get("kind").and_then(|v| v.as_str()))))
        .collect();
    let when: Vec<Value> = arr("when").iter()
        .map(|c| json!(n::nasdaq_when(c.get("row").unwrap_or(&Value::Null),
                                      c.get("now").and_then(|v| v.as_f64()).unwrap_or(now as f64) as i64)))
        .collect();
    let kinds: Vec<Value> = arr("kinds").iter().map(|s| json!(n::kind_of(s.as_str().unwrap_or("")))).collect();
    let clean: Vec<Value> = arr("clean").iter().map(|s| json!(n::clean_text(s.as_str().unwrap_or("")))).collect();
    let sources: Vec<Value> = arr("sources").iter()
        .map(|r| {
            let f = |i: usize| r.get(i).and_then(|v| v.as_str()).unwrap_or("");
            json!(n::source_for(f(0), f(1), f(2)))
        })
        .collect();
    println!("{}", serde_json::to_string(&json!({
        "tmx": tmx, "nasdaq": nasdaq, "when": when, "kinds": kinds, "clean": clean, "sources": sources,
    })).unwrap());
}
