//! The sync helpers that do not contact Wealthsimple, so they can be compared
//! with Python's on the same inputs.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    use bagholder_ws::sync as s;

    if mode == "bounds" {
        let path = doc.get("db").and_then(|v| v.as_str()).unwrap_or("");
        let conn = rusqlite::Connection::open(path).unwrap();
        let (start, full) = s::activity_sync_bounds(&conn).unwrap();
        println!("{}", serde_json::to_string(&json!({"start": start, "full": full,
            "incremental": s::incremental_start_date(&conn).unwrap()})).unwrap());
        return;
    }

    let now = doc.get("now").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let errors: Vec<Value> = doc.get("errors").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|e| json!(s::public_sync_error(e.as_str().unwrap_or("")))).collect();
    let sessions: Vec<Value> = doc.get("sessions").and_then(|v| v.as_array()).cloned().unwrap_or_default()
        .iter().map(|x| json!({"expiresAt": s::expires_at_unix(x), "needsRefresh": s::token_refresh_needed(x, now)})).collect();
    let accounts: Vec<Value> = doc.get("accounts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    println!("{}", serde_json::to_string(&json!({
        "errors": errors,
        "sessions": sessions,
        "slimAccounts": s::slim_accounts(&accounts),
        "boostTargets": accounts.iter().map(|a| json!(s::margin_boost_target(a))).collect::<Vec<_>>(),
    })).unwrap());
}
