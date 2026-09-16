//! Reads {snapshot, market, today, journal, filters} on stdin and writes the
//! whole `build_view` payload, so any section of it can be compared with
//! Python's.
use serde_json::Value;
use std::io::Read;

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    let snapshot = doc.get("snapshot").cloned().unwrap_or(Value::Null);
    let market = doc.get("market").cloned().unwrap_or(Value::Null);
    let today = doc.get("today").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let journal = doc.get("journal").and_then(|v| v.as_object()).cloned().unwrap_or_default();
    let base = bagholder_model::base::build_base(&snapshot, &market, &journal, Some(&today));
    let view = bagholder_model::view::build_view(&base, doc.get("filters"));
    println!("{}", serde_json::to_string(&view).unwrap());
}
