//! The fetch helpers that only shape a request or read a response, so they can
//! be compared with Python's without contacting Wealthsimple.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    let now = doc.get("now").and_then(|v| v.as_i64()).unwrap_or(0);
    use bagholder_ws::fetch as f;

    let conds: Vec<Value> = doc
        .get("conditions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|c| {
            let aid = c.get("accountId").and_then(|v| v.as_str()).unwrap_or("");
            let sd = c.get("startDate").and_then(|v| v.as_str());
            f::activity_fetch_condition(aid, sd, now)
        })
        .collect();

    let margins: Vec<Value> = doc
        .get("margins")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|m| f::parse_margin(m).unwrap_or(Value::Null))
        .collect();

    let accounts: Vec<Value> = doc.get("accounts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let moneys: Vec<Value> = doc
        .get("moneys")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|n| {
            let (a, c) = f::money_amount(n, &["netLiquidationValue", "netLiquidationValueV2", "netDeposits"]);
            json!([a, c])
        })
        .collect();

    println!("{}", serde_json::to_string(&json!({
        "conditions": conds,
        "margins": margins,
        "marginIds": f::margin_account_ids(&accounts),
        "moneys": moneys,
    })).unwrap());
}
