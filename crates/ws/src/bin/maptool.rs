//! Reads {items, accounts} on stdin and writes what the mapper makes of each
//! row, so the two implementations can be compared on the same feed.
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let doc: Value = serde_json::from_str(&buf).unwrap();
    let items: Vec<Value> = doc.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let accounts = doc.get("accounts").filter(|v| !v.is_null());

    let out: Vec<Value> = items
        .iter()
        .map(|it| {
            json!({
                "skip": bagholder_ws::mapping::skip_activity(it),
                "corp": bagholder_ws::mapping::is_corp_share_move(it),
                "codeChange": bagholder_ws::mapping::is_code_change(it),
                "assetSymbol": bagholder_ws::mapping::asset_symbol(it),
                "counterSymbol": bagholder_ws::mapping::counter_symbol(it),
                "optionSymbol": bagholder_ws::mapping::option_symbol(it),
                "signedCash": bagholder_ws::mapping::signed_cash(it),
                "rows": bagholder_ws::mapping::map_activity_rows(it, accounts),
            })
        })
        .collect();
    println!("{}", serde_json::to_string(&json!({
        "rows": out,
        "navGroups": bagholder_ws::mapping::nav_account_groups(accounts),
        "fifoPools": bagholder_ws::mapping::fifo_pool_ids(accounts),
    })).unwrap());
}
