//! The session helpers that do not touch the network, so they can be compared
//! with Python's on the same inputs.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    let now = doc.get("now").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rows: Vec<Value> = doc.get("rows").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    use bagholder_ws::session as s;
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "oauthError": s::oauth_error_code(r),
                "refreshMessage": s::refresh_failure_message(r),
                "expiresAt": s::expires_at_as_timestamp(r, now),
                "identity": s::identity_from(r),
                "clientId": s::client_id_from_token_info(r),
            })
        })
        .collect();
    println!("{}", serde_json::to_string(&out).unwrap());
}
