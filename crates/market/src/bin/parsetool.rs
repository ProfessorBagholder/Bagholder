//! Reads {fn, arg} rows on stdin and writes what each market parser makes of
//! them, so the two implementations can be compared on the same text.
use serde_json::{json, Value};
use std::io::Read;

use bagholder_market::parse;

fn main() {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).unwrap();
    let rows: Vec<Value> = serde_json::from_str(&buf).unwrap();
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            let f = r.get("fn").and_then(|v| v.as_str()).unwrap_or("");
            let arg = r.get("arg").cloned().unwrap_or(Value::Null);
            let text = arg.as_str().unwrap_or("").to_string();
            match f {
                "boc" => json!(parse::parse_boc_json(&text)),
                "fred" => json!(parse::parse_fred_csv(&text)),
                "stooq" => json!(parse::parse_stooq_csv(&text)),
                "tmx_history" => json!(parse::parse_tmx_history(&arg)),
                "cboe_ca_history" => json!(parse::parse_cboe_ca_history(&text)),
                "cboe_ca_quote" => parse::parse_cboe_ca_quote(&text).unwrap_or(Value::Null),
                "cboe_options" => json!(parse::parse_cboe_options(&text)),
                "option_mark" => parse::option_mark(&arg).unwrap_or(Value::Null),
                "coinbase" => {
                    let pair = r.get("pair").and_then(|v| v.as_str()).unwrap_or("");
                    parse::parse_coinbase_rec(&text, pair).unwrap_or(Value::Null)
                }
                "coinbase_candles" => json!(parse::parse_coinbase_candles(&text)),
                "occ" => json!(parse::occ_code(&text)),
                "yahoo_split" => {
                    let (t, v) = parse::yahoo_split(&text);
                    json!([t, v])
                }
                other => json!({"unknown": other}),
            }
        })
        .collect();
    println!("{}", serde_json::to_string(&out).unwrap());
}
